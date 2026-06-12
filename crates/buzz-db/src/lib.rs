#![deny(unsafe_code)]
#![warn(missing_docs)]
//! buzz-db — Postgres event store for Buzz.
//!
//! ## Design invariants
//! - AUTH events (kind 22242) are never stored — they carry bearer tokens.
//! - Ephemeral events (20000–29999) are never stored — Redis pub/sub only.
//! - Events table is partitioned by month on `created_at`.
//! - No FK references to partitioned tables.
//! - Uses `sqlx::query()` (runtime) not `sqlx::query!()` (compile-time).

/// API token storage and lookup.
pub mod api_token;
/// Relay-scoped archived identity persistence (NIP-IA).
pub mod archived_identities;
/// Channel and membership persistence.
pub mod channel;
/// Direct message channel persistence.
pub mod dm;
/// Database error types.
pub mod error;
/// Event storage and retrieval.
pub mod event;
/// Home feed queries.
pub mod feed;
/// Monthly table partition management.
pub mod partition;
/// Reaction persistence.
pub mod reaction;
/// Relay-level membership persistence (NIP-43).
pub mod relay_members;
/// Thread metadata persistence.
pub mod thread;
/// User profile persistence.
pub mod user;
/// Workflow, run, and approval persistence.
pub mod workflow;

pub use error::{DbError, Result};
pub use event::EventQuery;

use chrono::{DateTime, Utc};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, QueryBuilder, Row};
use std::time::Duration;
use uuid::Uuid;

use buzz_core::StoredEvent;

/// Extract p-tag mentions from an event and insert into the `event_mentions` table.
///
/// Called after event insertion. Failures are logged but do not block event storage.
/// Uses `INSERT ... ON CONFLICT DO NOTHING` so duplicate inserts are silently skipped.
pub async fn insert_mentions(
    pool: &PgPool,
    event: &nostr::Event,
    channel_id: Option<Uuid>,
) -> Result<()> {
    let p_tags: Vec<&str> = event
        .tags
        .iter()
        .filter_map(|tag| {
            let tag_vec = tag.as_slice();
            if tag_vec.len() >= 2 && tag_vec[0] == "p" {
                Some(tag_vec[1].as_str())
            } else {
                None
            }
        })
        .collect();

    if p_tags.is_empty() {
        return Ok(());
    }

    let event_id_bytes = event.id.as_bytes();
    let created_at_secs = event.created_at.as_secs() as i64;
    let created_at = DateTime::from_timestamp(created_at_secs, 0)
        .ok_or(crate::error::DbError::InvalidTimestamp(created_at_secs))?;
    let kind = event.kind.as_u16() as u32;

    // Validate and normalize pubkeys, logging any malformed ones.
    let valid_pubkeys: Vec<String> = p_tags
        .into_iter()
        .filter(|pk| {
            if pk.len() != 64 || !pk.chars().all(|c| c.is_ascii_hexdigit()) {
                tracing::debug!(
                    event_id = %event.id,
                    invalid_ptag = pk,
                    "skipping malformed p-tag in insert_mentions"
                );
                false
            } else {
                true
            }
        })
        .map(|pk| pk.to_ascii_lowercase())
        .collect();

    if valid_pubkeys.is_empty() {
        return Ok(());
    }

    // Single multi-row INSERT ... ON CONFLICT DO NOTHING — one round-trip regardless of mention count.
    let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(
        "INSERT INTO event_mentions \
         (pubkey_hex, event_id, event_created_at, channel_id, event_kind) ",
    );

    qb.push_values(&valid_pubkeys, |mut b, pubkey| {
        b.push_bind(pubkey.as_str())
            .push_bind(event_id_bytes.as_slice())
            .push_bind(created_at)
            .push_bind(channel_id)
            .push_bind(kind as i32);
    });

    qb.push(" ON CONFLICT DO NOTHING");

    qb.build().execute(pool).await?;
    Ok(())
}

/// Database handle. Clone is cheap (Arc-backed pool).
#[derive(Clone, Debug)]
pub struct Db {
    pub(crate) pool: PgPool,
}

/// Configuration for the Postgres connection pool.
#[derive(Debug, Clone)]
pub struct DbConfig {
    /// Postgres connection URL (e.g. `postgres://user:pass@host/db`).
    pub database_url: String,
    /// Maximum number of connections in the pool.
    pub max_connections: u32,
    /// Minimum number of idle connections to maintain.
    pub min_connections: u32,
    /// Seconds to wait when acquiring a connection before timing out.
    pub acquire_timeout_secs: u64,
    /// Maximum connection lifetime in seconds before recycling.
    pub max_lifetime_secs: u64,
    /// Seconds a connection may sit idle before being closed.
    pub idle_timeout_secs: u64,
}

impl Default for DbConfig {
    /// Sized for a single relay pod against PG max_connections=100.
    /// Staging measured 51 idle + 1 active out of 50 — most connections sat unused.
    /// At 20 main + 5 audit = 25/pod, four relay pods fit within the PG limit.
    fn default() -> Self {
        Self {
            database_url: "postgres://buzz:buzz_dev@localhost:5432/buzz".to_string(),
            max_connections: 20,
            min_connections: 2,
            acquire_timeout_secs: 3,
            max_lifetime_secs: 1800,
            idle_timeout_secs: 600,
        }
    }
}

/// Token summary returned by [`Db::list_active_tokens`].
#[derive(Debug, Clone)]
pub struct TokenSummary {
    /// Unique token identifier.
    pub id: Uuid,
    /// Human-readable token name.
    pub name: String,
    /// Compressed public key bytes of the token owner.
    pub owner_pubkey: Vec<u8>,
    /// Permission scopes granted to this token.
    pub scopes: Vec<String>,
    /// When the token was created.
    pub created_at: DateTime<Utc>,
    /// Optional expiry timestamp; `None` means no expiry.
    pub expires_at: Option<DateTime<Utc>>,
}

impl Db {
    /// Creates a new `Db` by connecting a Postgres pool with the given config.
    pub async fn new(config: &DbConfig) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(config.max_connections)
            .min_connections(config.min_connections)
            .acquire_timeout(Duration::from_secs(config.acquire_timeout_secs))
            .max_lifetime(Duration::from_secs(config.max_lifetime_secs))
            .idle_timeout(Duration::from_secs(config.idle_timeout_secs))
            .connect(&config.database_url)
            .await?;
        Ok(Self { pool })
    }

    /// Creates a `Db` from an existing `PgPool` (useful in tests).
    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Returns `true` if the database is reachable (used by readiness probes).
    pub async fn ping(&self) -> bool {
        sqlx::query("SELECT 1").execute(&self.pool).await.is_ok()
    }

    /// Begin a database transaction for atomic multi-statement operations.
    ///
    /// Returns a `'static` transaction because `PgPool` is `Arc`-backed internally.
    /// The transaction holds an owned pool handle, not a borrow.
    pub async fn begin_transaction(&self) -> Result<sqlx::Transaction<'static, sqlx::Postgres>> {
        self.pool.begin().await.map_err(Into::into)
    }

    // ── Events ───────────────────────────────────────────────────────────────

    /// Inserts an event. Returns `(StoredEvent, was_inserted)` — `false` on duplicate.
    pub async fn insert_event(
        &self,
        event: &nostr::Event,
        channel_id: Option<Uuid>,
    ) -> Result<(StoredEvent, bool)> {
        let result = event::insert_event(&self.pool, event, channel_id).await?;
        if result.1 {
            if let Err(e) = insert_mentions(&self.pool, event, channel_id).await {
                tracing::warn!(event_id = %event.id, "Failed to insert mentions: {e}");
            }
        }
        Ok(result)
    }

    /// Queries events matching the given filter parameters.
    pub async fn query_events(&self, q: &EventQuery) -> Result<Vec<StoredEvent>> {
        event::query_events(&self.pool, q).await
    }

    /// Count events matching the given query (NIP-45 COUNT support).
    pub async fn count_events(&self, q: &EventQuery) -> Result<i64> {
        event::count_events(&self.pool, q).await
    }

    /// Fetch the latest replaceable event for a (kind, pubkey) pair.
    ///
    /// Uses canonical NIP-16 ordering: `created_at DESC, id ASC`.
    /// This matches the write path in [`replace_addressable_event`] and handles
    /// historical duplicate survivors correctly.
    pub async fn get_latest_global_replaceable(
        &self,
        kind: i32,
        pubkey_bytes: &[u8],
    ) -> Result<Option<StoredEvent>> {
        event::get_latest_global_replaceable(&self.pool, kind, pubkey_bytes).await
    }

    /// Fetches a single non-deleted event by its raw ID bytes.
    ///
    /// Returns `None` if the event does not exist or has been soft-deleted.
    pub async fn get_event_by_id(&self, id_bytes: &[u8]) -> Result<Option<StoredEvent>> {
        event::get_event_by_id(&self.pool, id_bytes).await
    }

    /// Fetches a single event by its raw ID bytes, **including soft-deleted rows**.
    pub async fn get_event_by_id_including_deleted(
        &self,
        id_bytes: &[u8],
    ) -> Result<Option<StoredEvent>> {
        event::get_event_by_id_including_deleted(&self.pool, id_bytes).await
    }

    /// Soft-deletes an event. Returns `Ok(true)` if deleted, `Ok(false)` if already deleted.
    pub async fn soft_delete_event(&self, event_id: &[u8]) -> Result<bool> {
        event::soft_delete_event(&self.pool, event_id).await
    }

    /// Soft-delete the live row for an addressable coordinate `(kind, pubkey, d_tag)`.
    /// Used by NIP-09 a-tag deletion for parameterized-replaceable kinds.
    pub async fn soft_delete_by_coordinate(
        &self,
        kind: i32,
        pubkey: &[u8],
        d_tag: &str,
    ) -> Result<bool> {
        event::soft_delete_by_coordinate(&self.pool, kind, pubkey, d_tag).await
    }

    /// Atomically soft-delete an event and decrement thread reply counters.
    pub async fn soft_delete_event_and_update_thread(
        &self,
        event_id: &[u8],
        parent_event_id: Option<&[u8]>,
        root_event_id: Option<&[u8]>,
    ) -> Result<bool> {
        event::soft_delete_event_and_update_thread(
            &self.pool,
            event_id,
            parent_event_id,
            root_event_id,
        )
        .await
    }

    /// Returns the most recent `created_at` for a channel.
    pub async fn get_last_message_at(&self, channel_id: Uuid) -> Result<Option<DateTime<Utc>>> {
        event::get_last_message_at(&self.pool, channel_id).await
    }

    /// Bulk-fetch the most recent `created_at` for a set of channel IDs.
    pub async fn get_last_message_at_bulk(
        &self,
        channel_ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, DateTime<Utc>>> {
        event::get_last_message_at_bulk(&self.pool, channel_ids).await
    }

    /// Batch-fetch non-deleted events by their raw IDs.
    pub async fn get_events_by_ids(&self, ids: &[&[u8]]) -> Result<Vec<StoredEvent>> {
        event::get_events_by_ids(&self.pool, ids).await
    }

    /// Atomically insert an event AND its thread metadata in a single transaction.
    pub async fn insert_event_with_thread_metadata(
        &self,
        event: &nostr::Event,
        channel_id: Option<Uuid>,
        thread_meta: Option<event::ThreadMetadataParams<'_>>,
    ) -> Result<(StoredEvent, bool)> {
        let result =
            event::insert_event_with_thread_metadata(&self.pool, event, channel_id, thread_meta)
                .await?;
        if result.1 {
            if let Err(e) = insert_mentions(&self.pool, event, channel_id).await {
                tracing::warn!(event_id = %event.id, "Failed to insert mentions: {e}");
            }
        }
        Ok(result)
    }

    // ── Channels ─────────────────────────────────────────────────────────────

    /// Creates a new channel, bootstraps the creator as owner, and returns the record.
    pub async fn create_channel(
        &self,
        name: &str,
        channel_type: channel::ChannelType,
        visibility: channel::ChannelVisibility,
        description: Option<&str>,
        created_by: &[u8],
        ttl_seconds: Option<i32>,
    ) -> Result<channel::ChannelRecord> {
        channel::create_channel(
            &self.pool,
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
    pub async fn create_channel_with_id(
        &self,
        channel_id: Uuid,
        name: &str,
        channel_type: channel::ChannelType,
        visibility: channel::ChannelVisibility,
        description: Option<&str>,
        created_by: &[u8],
        ttl_seconds: Option<i32>,
    ) -> Result<(channel::ChannelRecord, bool)> {
        channel::create_channel_with_id(
            &self.pool,
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
    pub async fn get_channel(&self, channel_id: Uuid) -> Result<channel::ChannelRecord> {
        channel::get_channel(&self.pool, channel_id).await
    }

    /// Returns the canvas content for a channel, if any.
    pub async fn get_canvas(&self, channel_id: Uuid) -> Result<Option<String>> {
        channel::get_canvas(&self.pool, channel_id).await
    }

    /// Sets or clears the canvas content for a channel.
    pub async fn set_canvas(&self, channel_id: Uuid, canvas: Option<&str>) -> Result<()> {
        channel::set_canvas(&self.pool, channel_id, canvas).await
    }

    /// Adds a member to a channel.
    pub async fn add_member(
        &self,
        channel_id: Uuid,
        pubkey: &[u8],
        role: channel::MemberRole,
        invited_by: Option<&[u8]>,
    ) -> Result<channel::MemberRecord> {
        channel::add_member(&self.pool, channel_id, pubkey, role, invited_by).await
    }

    /// Removes a member from a channel.
    pub async fn remove_member(
        &self,
        channel_id: Uuid,
        pubkey: &[u8],
        actor_pubkey: &[u8],
    ) -> Result<()> {
        channel::remove_member(&self.pool, channel_id, pubkey, actor_pubkey).await
    }

    /// Returns `true` if the pubkey is an active member.
    pub async fn is_member(&self, channel_id: Uuid, pubkey: &[u8]) -> Result<bool> {
        channel::is_member(&self.pool, channel_id, pubkey).await
    }

    /// Returns all active members of a channel.
    pub async fn get_members(&self, channel_id: Uuid) -> Result<Vec<channel::MemberRecord>> {
        channel::get_members(&self.pool, channel_id).await
    }

    /// Returns active members for multiple channels in a single query.
    pub async fn get_members_bulk(
        &self,
        channel_ids: &[Uuid],
    ) -> Result<Vec<channel::MemberRecord>> {
        channel::get_members_bulk(&self.pool, channel_ids).await
    }

    /// Get all channel IDs accessible to a pubkey.
    pub async fn get_accessible_channel_ids(&self, pubkey: &[u8]) -> Result<Vec<Uuid>> {
        channel::get_accessible_channel_ids(&self.pool, pubkey).await
    }

    /// Lists channels, optionally filtered by visibility.
    pub async fn list_channels(
        &self,
        visibility: Option<&str>,
    ) -> Result<Vec<channel::ChannelRecord>> {
        channel::list_channels(&self.pool, visibility).await
    }

    /// Returns full channel records for all channels a user can access.
    pub async fn get_accessible_channels(
        &self,
        pubkey: &[u8],
        visibility_filter: Option<&str>,
        member_only: Option<bool>,
    ) -> Result<Vec<channel::AccessibleChannel>> {
        channel::get_accessible_channels(&self.pool, pubkey, visibility_filter, member_only).await
    }

    /// Returns all bot-role members with their aggregated channel names.
    pub async fn get_bot_members(&self) -> Result<Vec<channel::BotMemberRecord>> {
        channel::get_bot_members(&self.pool).await
    }

    /// Bulk-fetch user records by pubkey.
    pub async fn get_users_bulk(&self, pubkeys: &[Vec<u8>]) -> Result<Vec<channel::UserRecord>> {
        channel::get_users_bulk(&self.pool, pubkeys).await
    }

    /// Updates a channel's name and/or description.
    pub async fn update_channel(
        &self,
        channel_id: Uuid,
        updates: channel::ChannelUpdate,
    ) -> Result<channel::ChannelRecord> {
        channel::update_channel(&self.pool, channel_id, updates).await
    }

    /// Sets the topic for a channel.
    pub async fn set_topic(&self, channel_id: Uuid, topic: &str, set_by: &[u8]) -> Result<()> {
        channel::set_topic(&self.pool, channel_id, topic, set_by).await
    }

    /// Sets the purpose for a channel.
    pub async fn set_purpose(&self, channel_id: Uuid, purpose: &str, set_by: &[u8]) -> Result<()> {
        channel::set_purpose(&self.pool, channel_id, purpose, set_by).await
    }

    /// Archives a channel.
    pub async fn archive_channel(&self, channel_id: Uuid) -> Result<()> {
        channel::archive_channel(&self.pool, channel_id).await
    }

    /// Unarchives a channel.
    pub async fn unarchive_channel(&self, channel_id: Uuid) -> Result<()> {
        channel::unarchive_channel(&self.pool, channel_id).await
    }

    /// Soft-delete a channel.
    pub async fn soft_delete_channel(&self, channel_id: Uuid) -> Result<bool> {
        channel::soft_delete_channel(&self.pool, channel_id).await
    }

    /// Returns the count of active members in a channel.
    pub async fn get_member_count(&self, channel_id: Uuid) -> Result<i64> {
        channel::get_member_count(&self.pool, channel_id).await
    }

    /// Bulk-fetch member counts for a set of channel IDs.
    pub async fn get_member_counts_bulk(
        &self,
        channel_ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, i64>> {
        channel::get_member_counts_bulk(&self.pool, channel_ids).await
    }

    /// Get the active role of a pubkey in a channel.
    pub async fn get_member_role(&self, channel_id: Uuid, pubkey: &[u8]) -> Result<Option<String>> {
        channel::get_member_role(&self.pool, channel_id, pubkey).await
    }

    /// Bump the TTL deadline for an ephemeral channel after a new message.
    pub async fn bump_ttl_deadline(&self, channel_id: Uuid) -> Result<()> {
        channel::bump_ttl_deadline(&self.pool, channel_id).await
    }

    /// Archive ephemeral channels whose TTL deadline has passed.
    pub async fn reap_expired_ephemeral_channels(&self) -> Result<Vec<Uuid>> {
        channel::reap_expired_ephemeral_channels(&self.pool).await
    }

    // ── Users ────────────────────────────────────────────────────────────────

    /// Ensure a user record exists (upsert).
    pub async fn ensure_user(&self, pubkey: &[u8]) -> Result<()> {
        user::ensure_user(&self.pool, pubkey).await
    }

    /// Get a single user record by pubkey.
    pub async fn get_user(&self, pubkey: &[u8]) -> Result<Option<user::UserProfile>> {
        user::get_user(&self.pool, pubkey).await
    }

    /// Update a user's profile fields.
    pub async fn update_user_profile(
        &self,
        pubkey: &[u8],
        display_name: Option<&str>,
        avatar_url: Option<&str>,
        about: Option<&str>,
        nip05_handle: Option<&str>,
    ) -> Result<()> {
        user::update_user_profile(
            &self.pool,
            pubkey,
            display_name,
            avatar_url,
            about,
            nip05_handle,
        )
        .await
    }

    /// Look up a user by NIP-05 handle.
    pub async fn get_user_by_nip05(
        &self,
        local_part: &str,
        domain: &str,
    ) -> Result<Option<user::UserProfile>> {
        user::get_user_by_nip05(&self.pool, local_part, domain).await
    }

    /// Search users by display name, NIP-05 handle, or pubkey prefix.
    pub async fn search_users(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<user::UserSearchProfile>> {
        user::search_users(&self.pool, query, limit).await
    }

    /// Atomically set agent owner — only if no owner is currently assigned.
    /// Returns Ok(true) if set, Ok(false) if an owner already exists.
    pub async fn set_agent_owner(&self, agent_pubkey: &[u8], owner_pubkey: &[u8]) -> Result<bool> {
        user::set_agent_owner(&self.pool, agent_pubkey, owner_pubkey).await
    }

    /// Get the channel_add_policy and agent_owner_pubkey for a user.
    pub async fn get_agent_channel_policy(
        &self,
        pubkey: &[u8],
    ) -> Result<Option<(String, Option<Vec<u8>>)>> {
        user::get_agent_channel_policy(&self.pool, pubkey).await
    }

    /// Check whether `actor_pubkey` is the agent owner of `target_pubkey`.
    pub async fn is_agent_owner(&self, target_pubkey: &[u8], actor_pubkey: &[u8]) -> Result<bool> {
        user::is_agent_owner(&self.pool, target_pubkey, actor_pubkey).await
    }

    /// Set the channel_add_policy for a user.
    pub async fn set_channel_add_policy(&self, pubkey: &[u8], policy: &str) -> Result<()> {
        user::set_channel_add_policy(&self.pool, pubkey, policy).await
    }

    // ── Direct Messages ──────────────────────────────────────────────────────

    /// Find an existing DM by its participant hash.
    pub async fn find_dm_by_participants(
        &self,
        participant_hash: &[u8],
    ) -> Result<Option<channel::ChannelRecord>> {
        dm::find_dm_by_participants(&self.pool, participant_hash).await
    }

    /// Create or return an existing DM channel.
    pub async fn create_dm(
        &self,
        participants: &[&[u8]],
        created_by: &[u8],
    ) -> Result<channel::ChannelRecord> {
        dm::create_dm(&self.pool, participants, created_by).await
    }

    /// List all DMs for a user.
    pub async fn list_dms_for_user(
        &self,
        pubkey: &[u8],
        limit: u32,
        cursor: Option<Uuid>,
    ) -> Result<Vec<dm::DmRecord>> {
        dm::list_dms_for_user(&self.pool, pubkey, limit, cursor).await
    }

    /// Open or retrieve a DM for the given participants.
    pub async fn open_dm(
        &self,
        pubkeys: &[&[u8]],
        created_by: &[u8],
    ) -> Result<(channel::ChannelRecord, bool)> {
        dm::open_dm(&self.pool, pubkeys, created_by).await
    }

    /// Hide a DM channel for a specific user.
    ///
    /// The DM is not deleted — it can be restored by opening a new DM with
    /// the same participants.
    pub async fn hide_dm(&self, channel_id: Uuid, pubkey: &[u8]) -> Result<()> {
        dm::hide_dm(&self.pool, channel_id, pubkey).await
    }

    /// Unhide a DM channel for a specific user.
    pub async fn unhide_dm(&self, channel_id: Uuid, pubkey: &[u8]) -> Result<()> {
        dm::unhide_dm(&self.pool, channel_id, pubkey).await
    }

    /// List the channel IDs of all DMs the given user currently has hidden.
    pub async fn list_hidden_dms(&self, pubkey: &[u8]) -> Result<Vec<Uuid>> {
        dm::list_hidden_dms(&self.pool, pubkey).await
    }

    // ── Threads ──────────────────────────────────────────────────────────────

    /// Insert thread metadata.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_thread_metadata(
        &self,
        event_id: &[u8],
        event_created_at: DateTime<Utc>,
        channel_id: Uuid,
        parent_event_id: Option<&[u8]>,
        parent_event_created_at: Option<DateTime<Utc>>,
        root_event_id: Option<&[u8]>,
        root_event_created_at: Option<DateTime<Utc>>,
        depth: i32,
        broadcast: bool,
    ) -> Result<()> {
        thread::insert_thread_metadata(
            &self.pool,
            event_id,
            event_created_at,
            channel_id,
            parent_event_id,
            parent_event_created_at,
            root_event_id,
            root_event_created_at,
            depth,
            broadcast,
        )
        .await
    }

    /// Fetch replies under a root event.
    pub async fn get_thread_replies(
        &self,
        root_event_id: &[u8],
        depth_limit: Option<u32>,
        limit: u32,
        cursor: Option<&[u8]>,
    ) -> Result<Vec<thread::ThreadReply>> {
        thread::get_thread_replies(&self.pool, root_event_id, depth_limit, limit, cursor).await
    }

    /// Fetch aggregated thread stats.
    pub async fn get_thread_summary(
        &self,
        event_id: &[u8],
    ) -> Result<Option<thread::ThreadSummary>> {
        thread::get_thread_summary(&self.pool, event_id).await
    }

    /// Top-level messages for a channel.
    pub async fn get_channel_messages_top_level(
        &self,
        channel_id: Uuid,
        limit: u32,
        before_cursor: Option<DateTime<Utc>>,
        since_cursor: Option<DateTime<Utc>>,
        kind_filter: Option<&[u32]>,
    ) -> Result<Vec<thread::TopLevelMessage>> {
        thread::get_channel_messages_top_level(
            &self.pool,
            channel_id,
            limit,
            before_cursor,
            since_cursor,
            kind_filter,
        )
        .await
    }

    /// Look up a single thread_metadata row by event_id.
    pub async fn get_thread_metadata_by_event(
        &self,
        event_id: &[u8],
    ) -> Result<Option<thread::ThreadMetadataRecord>> {
        thread::get_thread_metadata_by_event(&self.pool, event_id).await
    }

    /// Decrement reply counts.
    pub async fn decrement_reply_count(
        &self,
        parent_event_id: &[u8],
        root_event_id: Option<&[u8]>,
    ) -> Result<()> {
        thread::decrement_reply_count(&self.pool, parent_event_id, root_event_id).await
    }

    // ── Reactions ────────────────────────────────────────────────────────────

    /// Add (or re-activate) a reaction.
    pub async fn add_reaction(
        &self,
        event_id: &[u8],
        event_created_at: DateTime<Utc>,
        pubkey: &[u8],
        emoji: &str,
        reaction_event_id: Option<&[u8]>,
    ) -> Result<bool> {
        reaction::add_reaction(
            &self.pool,
            event_id,
            event_created_at,
            pubkey,
            emoji,
            reaction_event_id,
        )
        .await
    }

    /// Soft-delete a reaction.
    pub async fn remove_reaction(
        &self,
        event_id: &[u8],
        event_created_at: DateTime<Utc>,
        pubkey: &[u8],
        emoji: &str,
    ) -> Result<bool> {
        reaction::remove_reaction(&self.pool, event_id, event_created_at, pubkey, emoji).await
    }

    /// Soft-delete a reaction by its source event ID.
    pub async fn remove_reaction_by_source_event_id(
        &self,
        reaction_event_id: &[u8],
    ) -> Result<bool> {
        reaction::remove_reaction_by_source_event_id(&self.pool, reaction_event_id).await
    }

    /// Look up the active reaction row for one actor + emoji + target tuple.
    pub async fn get_active_reaction_record(
        &self,
        event_id: &[u8],
        event_created_at: DateTime<Utc>,
        pubkey: &[u8],
        emoji: &str,
    ) -> Result<Option<reaction::ActiveReactionRecord>> {
        reaction::get_active_reaction_record(&self.pool, event_id, event_created_at, pubkey, emoji)
            .await
    }

    /// Backfill the source event ID on an active reaction row.
    pub async fn set_reaction_event_id(
        &self,
        event_id: &[u8],
        event_created_at: DateTime<Utc>,
        pubkey: &[u8],
        emoji: &str,
        reaction_event_id: &[u8],
    ) -> Result<bool> {
        reaction::set_reaction_event_id(
            &self.pool,
            event_id,
            event_created_at,
            pubkey,
            emoji,
            reaction_event_id,
        )
        .await
    }

    /// Get all active reactions for an event, grouped by emoji.
    pub async fn get_reactions(
        &self,
        event_id: &[u8],
        event_created_at: DateTime<Utc>,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<Vec<reaction::ReactionGroup>> {
        reaction::get_reactions(&self.pool, event_id, event_created_at, limit, cursor).await
    }

    /// Batch-fetch emoji counts for a set of (event_id, event_created_at) pairs.
    pub async fn get_reactions_bulk(
        &self,
        event_ids: &[(&[u8], DateTime<Utc>)],
    ) -> Result<Vec<reaction::BulkReactionEntry>> {
        reaction::get_reactions_bulk(&self.pool, event_ids).await
    }

    // ── Feed ─────────────────────────────────────────────────────────────────

    /// Find events that @mention the given pubkey.
    pub async fn query_feed_mentions(
        &self,
        pubkey_bytes: &[u8],
        accessible_channel_ids: &[Uuid],
        since: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<StoredEvent>> {
        feed::query_mentions(
            &self.pool,
            pubkey_bytes,
            accessible_channel_ids,
            since,
            limit,
        )
        .await
    }

    /// Find events that require action from the given pubkey.
    pub async fn query_feed_needs_action(
        &self,
        pubkey_bytes: &[u8],
        accessible_channel_ids: &[Uuid],
        since: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<StoredEvent>> {
        feed::query_needs_action(
            &self.pool,
            pubkey_bytes,
            accessible_channel_ids,
            since,
            limit,
        )
        .await
    }

    /// Find recent activity across accessible channels.
    pub async fn query_feed_activity(
        &self,
        accessible_channel_ids: &[Uuid],
        since: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<StoredEvent>> {
        feed::query_activity(&self.pool, accessible_channel_ids, since, limit).await
    }

    /// Find events that @mention the given pubkey (alias).
    pub async fn query_mentions(
        &self,
        pubkey_bytes: &[u8],
        accessible_channel_ids: &[Uuid],
        since: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<StoredEvent>> {
        feed::query_mentions(
            &self.pool,
            pubkey_bytes,
            accessible_channel_ids,
            since,
            limit,
        )
        .await
    }

    /// Find events that require action from the given pubkey.
    pub async fn query_needs_action(
        &self,
        pubkey_bytes: &[u8],
        accessible_channel_ids: &[Uuid],
        since: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<StoredEvent>> {
        feed::query_needs_action(
            &self.pool,
            pubkey_bytes,
            accessible_channel_ids,
            since,
            limit,
        )
        .await
    }

    /// Find recent activity across accessible channels.
    pub async fn query_activity(
        &self,
        accessible_channel_ids: &[Uuid],
        since: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<StoredEvent>> {
        feed::query_activity(&self.pool, accessible_channel_ids, since, limit).await
    }

    // ── API Tokens ───────────────────────────────────────────────────────────

    /// Create a new API token record.
    pub async fn create_api_token(
        &self,
        token_hash: &[u8],
        owner_pubkey: &[u8],
        name: &str,
        scopes: &[String],
        channel_ids: Option<&[Uuid]>,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<Uuid> {
        api_token::create_api_token(
            &self.pool,
            token_hash,
            owner_pubkey,
            name,
            scopes,
            channel_ids,
            expires_at,
        )
        .await
    }

    /// Atomic conditional INSERT with 10-token limit.
    pub async fn create_api_token_if_under_limit(
        &self,
        token_hash: &[u8],
        owner_pubkey: &[u8],
        name: &str,
        scopes: &[String],
        channel_ids: Option<&[Uuid]>,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<Option<Uuid>> {
        api_token::create_api_token_if_under_limit(
            &self.pool,
            token_hash,
            owner_pubkey,
            name,
            scopes,
            channel_ids,
            expires_at,
        )
        .await
    }

    /// Look up an active (non-revoked) API token by its SHA-256 hash.
    pub async fn get_api_token_by_hash(&self, hash: &[u8]) -> Result<Option<ApiTokenRecord>> {
        let row = sqlx::query(
            r#"
            SELECT id, token_hash, owner_pubkey, name, scopes, channel_ids,
                   created_at, expires_at, last_used_at, revoked_at
            FROM api_tokens
            WHERE token_hash = $1 AND revoked_at IS NULL
            "#,
        )
        .bind(hash)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            None => Ok(None),
            Some(r) => parse_api_token_row(r).map(Some),
        }
    }

    /// Look up an API token by hash, including revoked.
    pub async fn get_api_token_by_hash_including_revoked(
        &self,
        hash: &[u8],
    ) -> Result<Option<ApiTokenRecord>> {
        api_token::get_api_token_by_hash_including_revoked(&self.pool, hash).await
    }

    /// Record a token usage (update `last_used_at`).
    pub async fn touch_api_token(&self, hash: &[u8]) -> Result<()> {
        sqlx::query("UPDATE api_tokens SET last_used_at = NOW() WHERE token_hash = $1")
            .bind(hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Alias for [`touch_api_token`].
    pub async fn update_token_last_used(&self, hash: &[u8]) -> Result<()> {
        self.touch_api_token(hash).await
    }

    /// List all active (non-revoked) tokens, newest first.
    pub async fn list_active_tokens(&self) -> Result<Vec<TokenSummary>> {
        let rows = sqlx::query(
            r#"
            SELECT id, name, owner_pubkey, scopes, created_at, expires_at
            FROM api_tokens
            WHERE revoked_at IS NULL
            ORDER BY created_at DESC
            LIMIT 1000
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let id: Uuid = row.try_get("id")?;
            let scopes_json: serde_json::Value = row.try_get("scopes")?;
            let scopes: Vec<String> = serde_json::from_value(scopes_json)
                .map_err(|e| DbError::InvalidData(format!("scopes JSON: {e}")))?;

            out.push(TokenSummary {
                id,
                name: row.try_get("name")?,
                owner_pubkey: row.try_get("owner_pubkey")?,
                scopes,
                created_at: row.try_get("created_at")?,
                expires_at: row.try_get("expires_at")?,
            });
        }
        Ok(out)
    }

    /// List all tokens for a pubkey (including revoked).
    pub async fn list_tokens_by_owner(&self, pubkey: &[u8]) -> Result<Vec<ApiTokenRecord>> {
        api_token::list_tokens_by_owner(&self.pool, pubkey).await
    }

    /// Revoke a single token by ID.
    pub async fn revoke_token(
        &self,
        id: Uuid,
        owner_pubkey: &[u8],
        revoked_by: &[u8],
    ) -> Result<bool> {
        api_token::revoke_token(&self.pool, id, owner_pubkey, revoked_by).await
    }

    /// Revoke all active tokens for a pubkey.
    pub async fn revoke_all_tokens(&self, owner_pubkey: &[u8], revoked_by: &[u8]) -> Result<u64> {
        api_token::revoke_all_tokens(&self.pool, owner_pubkey, revoked_by).await
    }

    // ── Workflows ────────────────────────────────────────────────────────────

    /// Create a new workflow.
    pub async fn create_workflow(
        &self,
        channel_id: Option<Uuid>,
        owner_pubkey: &[u8],
        name: &str,
        definition_json: &str,
        definition_hash: &[u8],
    ) -> Result<Uuid> {
        workflow::create_workflow(
            &self.pool,
            channel_id,
            owner_pubkey,
            name,
            definition_json,
            definition_hash,
        )
        .await
    }

    /// Fetch a single workflow by ID.
    pub async fn get_workflow(&self, id: Uuid) -> Result<workflow::WorkflowRecord> {
        workflow::get_workflow(&self.pool, id).await
    }

    /// List workflows for a channel.
    pub async fn list_channel_workflows(
        &self,
        channel_id: Uuid,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Vec<workflow::WorkflowRecord>> {
        workflow::list_channel_workflows(&self.pool, channel_id, limit, offset).await
    }

    /// List active, enabled workflows for a channel.
    pub async fn list_enabled_channel_workflows(
        &self,
        channel_id: Uuid,
    ) -> Result<Vec<workflow::WorkflowRecord>> {
        workflow::list_enabled_channel_workflows(&self.pool, channel_id).await
    }

    /// List all active, enabled schedule-triggered workflows.
    pub async fn list_all_enabled_workflows(&self) -> Result<Vec<workflow::WorkflowRecord>> {
        workflow::list_all_enabled_workflows(&self.pool).await
    }

    /// Update a workflow's name, definition, and hash.
    pub async fn update_workflow(
        &self,
        id: Uuid,
        name: &str,
        definition_json: &str,
        definition_hash: &[u8],
    ) -> Result<()> {
        workflow::update_workflow(&self.pool, id, name, definition_json, definition_hash).await
    }

    /// Update a workflow's status.
    pub async fn update_workflow_status(
        &self,
        id: Uuid,
        status: workflow::WorkflowStatus,
    ) -> Result<()> {
        workflow::update_workflow_status(&self.pool, id, status).await
    }

    /// Enable or disable a workflow.
    pub async fn set_workflow_enabled(&self, id: Uuid, enabled: bool) -> Result<()> {
        workflow::set_workflow_enabled(&self.pool, id, enabled).await
    }

    /// Delete a workflow and all its runs/approvals.
    pub async fn delete_workflow(&self, id: Uuid) -> Result<()> {
        workflow::delete_workflow(&self.pool, id).await
    }

    /// Find a workflow by owner pubkey and name. Used for NIP-09 a-tag deletion
    /// where the d-tag is the workflow name (not UUID).
    pub async fn find_workflow_by_owner_and_name(
        &self,
        owner_pubkey: &[u8],
        name: &str,
    ) -> Result<Option<workflow::WorkflowRecord>> {
        workflow::find_by_owner_and_name(&self.pool, owner_pubkey, name).await
    }

    /// Create a new workflow run.
    pub async fn create_workflow_run(
        &self,
        workflow_id: Uuid,
        trigger_event_id: Option<&[u8]>,
        trigger_context: Option<&serde_json::Value>,
    ) -> Result<Uuid> {
        workflow::create_workflow_run(&self.pool, workflow_id, trigger_event_id, trigger_context)
            .await
    }

    /// Fetch a single workflow run.
    pub async fn get_workflow_run(&self, id: Uuid) -> Result<workflow::WorkflowRunRecord> {
        workflow::get_workflow_run(&self.pool, id).await
    }

    /// List runs for a workflow.
    pub async fn list_workflow_runs(
        &self,
        workflow_id: Uuid,
        limit: i64,
    ) -> Result<Vec<workflow::WorkflowRunRecord>> {
        workflow::list_workflow_runs(&self.pool, workflow_id, limit).await
    }

    /// Update a workflow run's status.
    pub async fn update_workflow_run(
        &self,
        id: Uuid,
        status: workflow::RunStatus,
        current_step: i32,
        trace: &serde_json::Value,
        error: Option<&str>,
    ) -> Result<()> {
        workflow::update_workflow_run(&self.pool, id, status, current_step, trace, error).await
    }

    /// Create an approval request.
    pub async fn create_approval(&self, params: workflow::CreateApprovalParams<'_>) -> Result<()> {
        workflow::create_approval(&self.pool, params).await
    }

    /// Fetch an approval by raw token.
    pub async fn get_approval(&self, token: &str) -> Result<workflow::ApprovalRecord> {
        workflow::get_approval(&self.pool, token).await
    }

    /// Fetch an approval by its already-hashed token (no re-hashing).
    pub async fn get_approval_by_stored_hash(
        &self,
        token_hash: &[u8],
    ) -> Result<workflow::ApprovalRecord> {
        workflow::get_approval_by_stored_hash(&self.pool, token_hash).await
    }

    /// Fetch all approvals for a workflow run.
    pub async fn get_run_approvals(
        &self,
        workflow_id: uuid::Uuid,
        run_id: uuid::Uuid,
    ) -> Result<Vec<workflow::ApprovalRecord>> {
        workflow::get_run_approvals(&self.pool, workflow_id, run_id).await
    }

    /// Update an approval's status.
    pub async fn update_approval(
        &self,
        token: &str,
        status: workflow::ApprovalStatus,
        approver_pubkey: Option<&[u8]>,
        note: Option<&str>,
    ) -> Result<bool> {
        workflow::update_approval(&self.pool, token, status, approver_pubkey, note).await
    }

    /// Update an approval by its already-hashed token (no re-hashing).
    pub async fn update_approval_by_stored_hash(
        &self,
        token_hash: &[u8],
        status: workflow::ApprovalStatus,
        approver_pubkey: Option<&[u8]>,
        note: Option<&str>,
    ) -> Result<bool> {
        workflow::update_approval_by_stored_hash(
            &self.pool,
            token_hash,
            status,
            approver_pubkey,
            note,
        )
        .await
    }

    // ── Partitions ──────────────────────────────────────────────────────────

    /// Ensures monthly partitions exist for the next N months.
    pub async fn ensure_future_partitions(&self, months_ahead: u32) -> Result<()> {
        partition::ensure_future_partitions(&self.pool, months_ahead).await
    }

    /// Backfill `d_tag` for existing NIP-33 events (kind 30000–39999) that have `d_tag IS NULL`.
    ///
    /// Idempotent — safe to call on every startup. No-ops when all rows are already populated.
    /// Runs a single UPDATE touching only NIP-33 rows with NULL d_tag.
    pub async fn backfill_d_tags(&self) -> Result<u64> {
        let result = sqlx::query(
            "UPDATE events \
             SET d_tag = COALESCE( \
                 (SELECT elem->>1 FROM jsonb_array_elements(tags) AS elem \
                  WHERE elem->>0 = 'd' LIMIT 1), \
                 '' \
             ) \
             WHERE kind BETWEEN 30000 AND 39999 AND d_tag IS NULL",
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    // ── Pubkey Allowlist ─────────────────────────────────────────────────────

    /// Check if a pubkey is in the allowlist.
    pub async fn is_pubkey_allowed(&self, pubkey: &[u8]) -> Result<bool> {
        let row = sqlx::query("SELECT COUNT(*) as cnt FROM pubkey_allowlist WHERE pubkey = $1")
            .bind(pubkey)
            .fetch_one(&self.pool)
            .await?;
        let cnt: i64 = row.try_get("cnt")?;
        Ok(cnt > 0)
    }

    /// Check if the allowlist has any entries (i.e. is enforcement active).
    pub async fn has_allowlist_entries(&self) -> Result<bool> {
        let row = sqlx::query("SELECT COUNT(*) as cnt FROM pubkey_allowlist")
            .fetch_one(&self.pool)
            .await?;
        let cnt: i64 = row.try_get("cnt")?;
        Ok(cnt > 0)
    }

    /// Add a pubkey to the allowlist.
    pub async fn add_to_allowlist(
        &self,
        pubkey: &[u8],
        added_by: &[u8],
        note: Option<&str>,
    ) -> Result<bool> {
        let result = sqlx::query(
            "INSERT INTO pubkey_allowlist (pubkey, added_by, note) VALUES ($1, $2, $3) \
             ON CONFLICT DO NOTHING",
        )
        .bind(pubkey)
        .bind(added_by)
        .bind(note)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Remove a pubkey from the allowlist.
    pub async fn remove_from_allowlist(&self, pubkey: &[u8]) -> Result<bool> {
        let result = sqlx::query("DELETE FROM pubkey_allowlist WHERE pubkey = $1")
            .bind(pubkey)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// List all pubkeys in the allowlist.
    pub async fn list_allowlist(&self) -> Result<Vec<AllowlistEntry>> {
        let rows = sqlx::query(
            "SELECT pubkey, added_by, added_at, note FROM pubkey_allowlist ORDER BY added_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(AllowlistEntry {
                pubkey: row.try_get("pubkey")?,
                added_by: row.try_get("added_by")?,
                added_at: row.try_get("added_at")?,
                note: row.try_get("note")?,
            });
        }
        Ok(out)
    }

    // ── Relay Members (NIP-43) ───────────────────────────────────────────────

    /// Returns `true` if `pubkey` (64-char hex) is in the relay member list.
    pub async fn is_relay_member(&self, pubkey: &str) -> Result<bool> {
        relay_members::is_relay_member(&self.pool, pubkey).await
    }

    /// Returns the relay member record for `pubkey`, or `None` if not found.
    pub async fn get_relay_member(
        &self,
        pubkey: &str,
    ) -> Result<Option<relay_members::RelayMember>> {
        relay_members::get_relay_member(&self.pool, pubkey).await
    }

    /// Returns all relay members ordered by `created_at` ascending.
    pub async fn list_relay_members(&self) -> Result<Vec<relay_members::RelayMember>> {
        relay_members::list_relay_members(&self.pool).await
    }

    /// Adds a new relay member. No-ops silently if the pubkey already exists (idempotent).
    /// Adds a new relay member.
    ///
    /// Returns `true` if the row was actually inserted, `false` if the pubkey
    /// already existed (idempotent — `ON CONFLICT DO NOTHING`).
    pub async fn add_relay_member(
        &self,
        pubkey: &str,
        role: &str,
        added_by: Option<&str>,
    ) -> Result<bool> {
        relay_members::add_relay_member(&self.pool, pubkey, role, added_by).await
    }

    /// Removes a relay member atomically, refusing to delete the owner.
    pub async fn remove_relay_member(&self, pubkey: &str) -> Result<relay_members::RemoveResult> {
        relay_members::remove_relay_member(&self.pool, pubkey).await
    }

    /// Removes a relay member only if their current role matches `expected_role`.
    ///
    /// Atomic conditional delete — eliminates the TOCTOU race between a
    /// prior role read and the delete. See [`relay_members::remove_relay_member_if_role`].
    pub async fn remove_relay_member_if_role(
        &self,
        pubkey: &str,
        expected_role: &str,
    ) -> Result<relay_members::RemoveResult> {
        relay_members::remove_relay_member_if_role(&self.pool, pubkey, expected_role).await
    }

    /// Updates the role of an existing relay member. Returns `true` if updated.
    pub async fn update_relay_member_role(&self, pubkey: &str, new_role: &str) -> Result<bool> {
        relay_members::update_relay_member_role(&self.pool, pubkey, new_role).await
    }

    /// Ensures the owner pubkey exists with role `"owner"`. Called at startup.
    pub async fn bootstrap_owner(&self, owner_pubkey: &str) -> Result<()> {
        relay_members::bootstrap_owner(&self.pool, owner_pubkey).await
    }

    /// Migrates existing `pubkey_allowlist` entries into `relay_members`.
    ///
    /// Idempotent — uses `ON CONFLICT DO NOTHING`. Returns the number of rows
    /// inserted, or 0 if the `pubkey_allowlist` table doesn't exist.
    pub async fn backfill_from_allowlist(&self) -> Result<u64> {
        relay_members::backfill_from_allowlist(&self.pool).await
    }

    // ── Archived identities (NIP-IA) ──────────────────────────────────────────

    /// Returns `true` if `pubkey` (64-char hex) is currently archived.
    pub async fn is_archived(&self, pubkey: &str) -> Result<bool> {
        archived_identities::is_archived(&self.pool, pubkey).await
    }

    /// Archives an identity. Returns `true` if inserted, `false` if already archived.
    pub async fn archive(
        &self,
        pubkey: &str,
        consent_path: &str,
        actor: &str,
        reason: Option<&str>,
        replaced_by: Option<&str>,
        request_event_id: &str,
    ) -> Result<bool> {
        archived_identities::archive(
            &self.pool,
            pubkey,
            consent_path,
            actor,
            reason,
            replaced_by,
            request_event_id,
        )
        .await
    }

    /// Unarchives an identity. Returns `true` if deleted, `false` if absent.
    pub async fn unarchive(&self, pubkey: &str) -> Result<bool> {
        archived_identities::unarchive(&self.pool, pubkey).await
    }

    /// Returns all archived identities ordered by archive time ascending.
    pub async fn list_archived(&self) -> Result<Vec<archived_identities::ArchivedIdentity>> {
        archived_identities::list_archived(&self.pool).await
    }

    // ── Discovery events ─────────────────────────────────────────────────────

    /// Soft-delete NIP-29 discovery events for a channel created by a specific relay pubkey.
    pub async fn soft_delete_discovery_events(
        &self,
        channel_id: Uuid,
        relay_pubkey: &[u8],
    ) -> Result<u64> {
        let result = sqlx::query(
            "UPDATE events SET deleted_at = NOW() \
             WHERE channel_id = $1 AND pubkey = $2 AND deleted_at IS NULL AND kind IN (39000, 39001, 39002)",
        )
        .bind(channel_id)
        .bind(relay_pubkey)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    // ── Replaceable events ─────────────────────────────────────────────────

    /// Atomically replace a replaceable event: NIP-16 kinds (0, 3, 41, 10000–19999)
    /// and NIP-29 discovery state (39000–39002, called from side_effects.rs).
    ///
    /// Keeps only the event with the highest `created_at` per (kind, pubkey, channel_id).
    /// Same-second ties are broken by lowest event `id` (NIP-16 deterministic ordering).
    /// Returns `(event, false)` for stale writes and duplicate IDs — callers should
    /// skip fan-out/dispatch when `was_inserted` is false.
    pub async fn replace_addressable_event(
        &self,
        event: &nostr::Event,
        channel_id: Option<Uuid>,
    ) -> Result<(StoredEvent, bool)> {
        let kind_i32 = buzz_core::kind::event_kind_i32(event);
        let pubkey_bytes = event.pubkey.to_bytes();
        let created_at_secs = event.created_at.as_secs() as i64;
        let created_at = chrono::DateTime::from_timestamp(created_at_secs, 0)
            .ok_or(DbError::InvalidTimestamp(created_at_secs))?;

        // Stable advisory-lock key: hash (kind, pubkey, channel_id) to i64.
        // Uses FNV-1a for determinism — Rust's DefaultHasher is NOT stable across processes.
        // Collisions cause extra serialization, not incorrect behavior.
        let lock_key = {
            let mut h: u64 = 0xcbf29ce484222325; // FNV offset basis
            for b in kind_i32.to_le_bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x100000001b3); // FNV prime
            }
            for b in pubkey_bytes.as_slice() {
                h ^= *b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
            if let Some(ch) = channel_id {
                for b in ch.as_bytes() {
                    h ^= *b as u64;
                    h = h.wrapping_mul(0x100000001b3);
                }
            }
            h as i64
        };

        let mut tx = self.pool.begin().await?;

        // Serialize all writers for the same (kind, pubkey, channel_id) tuple.
        // Advisory lock is transaction-scoped — released on commit/rollback.
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock_key)
            .execute(&mut *tx)
            .await?;

        // Check for the newest existing event. ORDER BY + LIMIT 1 is defensive against
        // historical data where prior bugs may have left multiple live rows.
        let existing: Option<(chrono::DateTime<chrono::Utc>, Vec<u8>)> = sqlx::query_as(
            "SELECT created_at, id FROM events \
             WHERE kind = $1 AND pubkey = $2 \
             AND channel_id IS NOT DISTINCT FROM $3 \
             AND deleted_at IS NULL \
             ORDER BY created_at DESC, id ASC LIMIT 1",
        )
        .bind(kind_i32)
        .bind(pubkey_bytes.as_slice())
        .bind(channel_id)
        .fetch_optional(&mut *tx)
        .await?;

        // Stale-write protection: reject if incoming is not newer.
        // NIP-16: created_at is second-resolution. On same-second tie, lowest
        // event id (lexicographic) wins — deterministic across relays.
        let incoming_id = event.id.as_bytes().as_slice();
        if let Some((existing_ts, existing_id)) = existing {
            let dominated = created_at < existing_ts
                || (created_at == existing_ts && incoming_id >= existing_id.as_slice());
            if dominated {
                tx.rollback().await?;
                let received_at = chrono::Utc::now();
                return Ok((
                    StoredEvent::with_received_at(event.clone(), received_at, channel_id, false),
                    false,
                ));
            }
        }

        // Soft-delete the old event (if any). IS NOT DISTINCT FROM for NULL safety.
        sqlx::query(
            "UPDATE events SET deleted_at = NOW() \
             WHERE kind = $1 AND pubkey = $2 \
             AND channel_id IS NOT DISTINCT FROM $3 \
             AND deleted_at IS NULL",
        )
        .bind(kind_i32)
        .bind(pubkey_bytes.as_slice())
        .bind(channel_id)
        .execute(&mut *tx)
        .await?;

        // Insert the new event inside the same transaction.
        let sig_bytes = event.sig.serialize();
        let tags_json = serde_json::to_value(&event.tags)?;
        let received_at = chrono::Utc::now();
        let d_tag = crate::event::extract_d_tag(event);

        let insert_result = sqlx::query(
            "INSERT INTO events (id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id, d_tag) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
             ON CONFLICT DO NOTHING",
        )
        .bind(event.id.as_bytes().as_slice())
        .bind(pubkey_bytes.as_slice())
        .bind(created_at)
        .bind(kind_i32)
        .bind(&tags_json)
        .bind(&event.content)
        .bind(sig_bytes.as_slice())
        .bind(received_at)
        .bind(channel_id)
        .bind(d_tag.as_deref())
        .execute(&mut *tx)
        .await?;

        let was_inserted = insert_result.rows_affected() > 0;
        if !was_inserted {
            // ON CONFLICT fired — the event ID already exists. Rollback the
            // soft-delete so we don't lose the previous replaceable event.
            tx.rollback().await?;
            return Ok((
                StoredEvent::with_received_at(event.clone(), received_at, channel_id, false),
                false,
            ));
        }

        tx.commit().await?;

        // Mentions are a denormalized index — safe outside the transaction.
        // insert_event() normally handles this, but we inlined the INSERT above.
        if let Err(e) = crate::insert_mentions(&self.pool, event, channel_id).await {
            tracing::warn!(event_id = %event.id, "Failed to insert mentions: {e}");
        }

        Ok((
            StoredEvent::with_received_at(event.clone(), received_at, channel_id, true),
            true,
        ))
    }

    /// Atomically replace a NIP-33 parameterized replaceable event (kind 30000–39999).
    ///
    /// Keeps only the event with the highest `created_at` per `(kind, pubkey, d_tag)`.
    /// Same-second ties are broken by lowest event `id` (deterministic ordering).
    /// The entire check → soft-delete → insert runs in a single transaction with
    /// an advisory lock to prevent concurrent-insert races.
    ///
    /// **Channel policy:** NIP-33 replacement keys on `(kind, pubkey, d_tag)` globally —
    /// `channel_id` is NOT part of the replacement key. This matches the Nostr spec:
    /// an author's parameterized replaceable event is a single global resource identified
    /// by its d-tag, regardless of which channel it was submitted to. The `channel_id`
    /// parameter is stored on the new row for query scoping but does not affect replacement.
    ///
    /// Note: `replace_addressable_event()` keys on `channel_id` because it serves
    /// relay-signed NIP-29 group metadata (kind 39000–39002) where the relay is the
    /// author and channel_id distinguishes groups. User-submitted NIP-33 events use
    /// this function instead, where the author's pubkey + d-tag is the natural key.
    pub async fn replace_parameterized_event(
        &self,
        event: &nostr::Event,
        d_tag: &str,
        channel_id: Option<Uuid>,
    ) -> Result<(StoredEvent, bool)> {
        let kind_i32 = buzz_core::kind::event_kind_i32(event);
        let pubkey_bytes = event.pubkey.to_bytes();
        let created_at_secs = event.created_at.as_secs() as i64;
        let created_at = chrono::DateTime::from_timestamp(created_at_secs, 0)
            .ok_or(DbError::InvalidTimestamp(created_at_secs))?;

        // Stable advisory-lock key: FNV-1a over (kind, pubkey, d_tag).
        // Same algorithm as replace_addressable_event — deterministic across processes.
        let lock_key = {
            let mut h: u64 = 0xcbf29ce484222325; // FNV offset basis
            for b in kind_i32.to_le_bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
            for b in pubkey_bytes.as_slice() {
                h ^= *b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
            for b in d_tag.as_bytes() {
                h ^= *b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
            h as i64
        };

        let mut tx = self.pool.begin().await?;

        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock_key)
            .execute(&mut *tx)
            .await?;

        // Check for existing event with same (kind, pubkey, d_tag).
        let existing: Option<(chrono::DateTime<chrono::Utc>, Vec<u8>)> = sqlx::query_as(
            "SELECT created_at, id FROM events \
             WHERE kind = $1 AND pubkey = $2 AND d_tag = $3 AND deleted_at IS NULL \
             ORDER BY created_at DESC, id ASC LIMIT 1",
        )
        .bind(kind_i32)
        .bind(pubkey_bytes.as_slice())
        .bind(d_tag)
        .fetch_optional(&mut *tx)
        .await?;

        // Stale-write protection: reject if incoming is not newer.
        let incoming_id = event.id.as_bytes().as_slice();
        if let Some((existing_ts, existing_id)) = existing {
            let dominated = created_at < existing_ts
                || (created_at == existing_ts && incoming_id >= existing_id.as_slice());
            if dominated {
                tx.rollback().await?;
                let received_at = chrono::Utc::now();
                return Ok((
                    StoredEvent::with_received_at(event.clone(), received_at, channel_id, false),
                    false,
                ));
            }

            // Soft-delete the older event(s).
            sqlx::query(
                "UPDATE events SET deleted_at = NOW() \
                 WHERE kind = $1 AND pubkey = $2 AND d_tag = $3 AND deleted_at IS NULL",
            )
            .bind(kind_i32)
            .bind(pubkey_bytes.as_slice())
            .bind(d_tag)
            .execute(&mut *tx)
            .await?;
        }

        // Insert the new event inside the transaction.
        let sig_bytes = event.sig.serialize();
        let tags_json = serde_json::to_value(&event.tags)?;
        let received_at = chrono::Utc::now();

        let insert_result = sqlx::query(
            "INSERT INTO events (id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id, d_tag) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
             ON CONFLICT DO NOTHING",
        )
        .bind(event.id.as_bytes().as_slice())
        .bind(pubkey_bytes.as_slice())
        .bind(created_at)
        .bind(kind_i32)
        .bind(&tags_json)
        .bind(&event.content)
        .bind(sig_bytes.as_slice())
        .bind(received_at)
        .bind(channel_id)
        .bind(d_tag)
        .execute(&mut *tx)
        .await?;

        let was_inserted = insert_result.rows_affected() > 0;
        if !was_inserted {
            tx.rollback().await?;
            return Ok((
                StoredEvent::with_received_at(event.clone(), received_at, channel_id, false),
                false,
            ));
        }

        tx.commit().await?;

        // Mentions are a denormalized index — safe outside the transaction.
        if let Err(e) = crate::insert_mentions(&self.pool, event, channel_id).await {
            tracing::warn!(event_id = %event.id, "Failed to insert mentions: {e}");
        }

        Ok((
            StoredEvent::with_received_at(event.clone(), received_at, channel_id, true),
            true,
        ))
    }
}

/// A full API token record.
#[derive(Debug, Clone)]
pub struct ApiTokenRecord {
    /// Unique token identifier.
    pub id: Uuid,
    /// SHA-256 hash of the raw token value.
    pub token_hash: Vec<u8>,
    /// Compressed public key bytes of the token owner.
    pub owner_pubkey: Vec<u8>,
    /// Human-readable token name.
    pub name: String,
    /// Permission scopes granted to this token.
    pub scopes: Vec<String>,
    /// Optional channel ID restrictions.
    pub channel_ids: Option<Vec<Uuid>>,
    /// When the token was created.
    pub created_at: DateTime<Utc>,
    /// Optional expiry timestamp.
    pub expires_at: Option<DateTime<Utc>>,
    /// When the token was last used.
    pub last_used_at: Option<DateTime<Utc>>,
    /// When the token was revoked.
    pub revoked_at: Option<DateTime<Utc>>,
}

/// An entry in the pubkey allowlist.
#[derive(Debug, Clone)]
pub struct AllowlistEntry {
    /// The allowed pubkey.
    pub pubkey: Vec<u8>,
    /// Who added this entry.
    pub added_by: Vec<u8>,
    /// When the entry was added.
    pub added_at: DateTime<Utc>,
    /// Optional note.
    pub note: Option<String>,
}

fn parse_api_token_row(row: sqlx::postgres::PgRow) -> Result<ApiTokenRecord> {
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

    Ok(ApiTokenRecord {
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
    })
}
