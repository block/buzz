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
/// Git repository name registry (NIP-34 kind:30617).
pub mod git_repo;
/// Embedded database migrations.
pub mod migration;
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
pub use event::{EventQuery, ReactionEventInsertOutcome};

use chrono::{DateTime, Utc};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, QueryBuilder, Row};
use std::time::Duration;
use uuid::Uuid;

use buzz_core::{CommunityId, StoredEvent};

/// Extract p-tag mentions from an event and insert into the `event_mentions` table.
///
/// Called after event insertion. Failures are logged but do not block event storage.
/// Uses `INSERT ... ON CONFLICT DO NOTHING` so duplicate inserts are silently skipped.
pub async fn insert_mentions(
    pool: &PgPool,
    community_id: CommunityId,
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
         (community_id, pubkey_hex, event_id, event_created_at, channel_id, event_kind) ",
    );

    qb.push_values(&valid_pubkeys, |mut b, pubkey| {
        b.push_bind(community_id.as_uuid())
            .push_bind(pubkey.as_str())
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
    /// Maximum connections configured for this pool (from [`DbConfig::max_connections`]).
    pub(crate) max_connections: u32,
}

/// Snapshot of Postgres connection pool utilisation.
#[derive(Debug, Clone, Copy)]
pub struct DbPoolStats {
    /// Total connections currently in the pool (idle + active).
    pub size: u32,
    /// Connections available for immediate reuse.
    pub idle: u32,
    /// Pool ceiling — the `max_connections` value set at construction.
    pub max: u32,
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

/// Community host-map row returned by [`Db::lookup_community_by_host`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommunityRecord {
    /// Stable server-resolved community id.
    pub id: CommunityId,
    /// Normalized host that maps to this community.
    pub host: String,
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
        Ok(Self {
            pool,
            max_connections: config.max_connections,
        })
    }

    /// Creates a `Db` from an existing `PgPool` (useful in tests).
    pub fn from_pool(pool: PgPool) -> Self {
        Self {
            max_connections: pool.options().get_max_connections(),
            pool,
        }
    }

    /// Run pending database migrations.
    pub async fn migrate(&self) -> Result<()> {
        migration::run_migrations(&self.pool).await
    }

    /// Returns `true` if the database is reachable (used by readiness probes).
    pub async fn ping(&self) -> bool {
        sqlx::query("SELECT 1").execute(&self.pool).await.is_ok()
    }

    /// Returns pool utilisation stats for metrics emission.
    ///
    /// `size`  — total connections (idle + active)
    /// `idle`  — connections available for immediate reuse
    /// `max`   — pool ceiling set at construction
    pub fn pool_stats(&self) -> DbPoolStats {
        DbPoolStats {
            size: self.pool.size(),
            idle: self.pool.num_idle() as u32,
            max: self.max_connections,
        }
    }

    /// Begin a database transaction for atomic multi-statement operations.
    ///
    /// Returns a `'static` transaction because `PgPool` is `Arc`-backed internally.
    /// The transaction holds an owned pool handle, not a borrow.
    pub async fn begin_transaction(&self) -> Result<sqlx::Transaction<'static, sqlx::Postgres>> {
        self.pool.begin().await.map_err(Into::into)
    }

    /// Returns the community mapped to a normalized request host, if one exists.
    ///
    /// The caller owns host normalization and turns `None` into the fail-closed
    /// request/connection error. buzz-db only reads the durable host map.
    pub async fn lookup_community_by_host(
        &self,
        normalized_host: &str,
    ) -> Result<Option<CommunityRecord>> {
        let row = sqlx::query(
            r#"
            SELECT id, host
            FROM communities
            WHERE lower(host) = lower($1)
            "#,
        )
        .bind(normalized_host)
        .fetch_optional(&self.pool)
        .await?;

        row.map(|row| {
            let id: Uuid = row.try_get("id")?;
            let host: String = row.try_get("host")?;

            Ok(CommunityRecord {
                id: CommunityId::from_uuid(id),
                host,
            })
        })
        .transpose()
    }

    /// Returns the normalized host mapped to a community id, if the community
    /// exists.
    ///
    /// The reverse of [`lookup_community_by_host`]: used by side-effect
    /// producers that already hold a server-resolved `CommunityId` (e.g. the
    /// workflow action sink running a run owned by some community) and need a
    /// fully-formed [`buzz_core::tenant::TenantContext`] — host included — to
    /// fan out under *that* community rather than the deployment default. The
    /// community is authoritative; the host is read back for labelling only and
    /// is never used to re-derive the community.
    pub async fn lookup_community_host(&self, community_id: CommunityId) -> Result<Option<String>> {
        let row = sqlx::query(
            r#"
            SELECT host
            FROM communities
            WHERE id = $1
            "#,
        )
        .bind(community_id.as_uuid())
        .fetch_optional(&self.pool)
        .await?;

        row.map(|row| {
            let host: String = row.try_get("host")?;
            Ok(host)
        })
        .transpose()
    }

    /// Returns the community's workspace icon (NIP-11 `icon`), if set.
    ///
    /// Set by relay admins/owners via the kind:9033 command; the value is
    /// validated and size-capped at that write path.
    pub async fn get_community_icon(&self, community_id: CommunityId) -> Result<Option<String>> {
        let row = sqlx::query(
            r#"
            SELECT icon
            FROM communities
            WHERE id = $1
            "#,
        )
        .bind(community_id.as_uuid())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row
            .map(|row| row.try_get::<Option<String>, _>("icon"))
            .transpose()?
            .flatten()
            .filter(|icon| !icon.is_empty()))
    }

    /// Sets or clears (`None`) the community's workspace icon.
    pub async fn set_community_icon(
        &self,
        community_id: CommunityId,
        icon: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE communities
            SET icon = $2
            WHERE id = $1
            "#,
        )
        .bind(community_id.as_uuid())
        .bind(icon)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Ensure a configured community host exists and return its row.
    ///
    /// This is the startup/config seeding path for N=1 deployments. Migrations
    /// create the schema only; deployment-specific hosts are not hardcoded into
    /// schema history.
    pub async fn ensure_configured_community(
        &self,
        normalized_host: &str,
    ) -> Result<CommunityRecord> {
        let row = sqlx::query(
            r#"
            INSERT INTO communities (host)
            VALUES ($1)
            ON CONFLICT (lower(host)) DO UPDATE SET host = EXCLUDED.host
            RETURNING id, host
            "#,
        )
        .bind(normalized_host)
        .fetch_one(&self.pool)
        .await?;

        let id: Uuid = row.try_get("id")?;
        let host: String = row.try_get("host")?;

        Ok(CommunityRecord {
            id: CommunityId::from_uuid(id),
            host,
        })
    }

    /// Returns the community that owns a channel, if the channel exists.
    ///
    /// Internal relay producers use this to derive tenant context from the row
    /// they are acting on, rather than falling back to an implicit default.
    pub async fn community_of_channel(&self, channel_id: Uuid) -> Result<Option<CommunityId>> {
        let row = sqlx::query(
            r#"
            SELECT community_id
            FROM channels
            WHERE id = $1
              AND deleted_at IS NULL
            "#,
        )
        .bind(channel_id)
        .fetch_optional(&self.pool)
        .await?;

        row.map(|row| {
            let id: Uuid = row.try_get("community_id")?;
            Ok(CommunityId::from_uuid(id))
        })
        .transpose()
    }

    /// Batched version of [`Self::community_of_channel`]: given a list of
    /// channel UUIDs, returns a map from channel id → owning community
    /// for every channel that exists (soft-deletes excluded).
    ///
    /// Used by the runtime conformance read-seam emitters in `buzz-relay`:
    /// after a `query_events`/`get_events_by_ids` returns N rows, the
    /// emitter collects distinct `channel_id`s, calls this once, then
    /// projects each row's true community label independently of the
    /// fetch query's WHERE clause. That independence is what makes the
    /// `Inv_NonInterference` / `Inv_ReadConfinement` gate non-vacuous —
    /// a mutation that dropped `community_id = $X` from the fetch query
    /// would still let this helper return the row's true label, and the
    /// checker would see the mismatch.
    ///
    /// Channels missing from the result map (deleted or never existed)
    /// are intentionally not present rather than mapped to a default —
    /// callers MUST treat "channel-id not in map" as a coverage breach,
    /// never as "use the resolved community".
    pub async fn communities_of_channels(
        &self,
        channel_ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, CommunityId>> {
        if channel_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let rows = sqlx::query(
            r#"
            SELECT id, community_id
            FROM channels
            WHERE id = ANY($1)
              AND deleted_at IS NULL
            "#,
        )
        .bind(channel_ids)
        .fetch_all(&self.pool)
        .await?;

        let mut out = std::collections::HashMap::with_capacity(rows.len());
        for row in rows {
            let ch: Uuid = row.try_get("id")?;
            let cm: Uuid = row.try_get("community_id")?;
            out.insert(ch, CommunityId::from_uuid(cm));
        }
        Ok(out)
    }

    /// Inserts an event. Returns `(StoredEvent, was_inserted)` — `false` on duplicate.
    pub async fn insert_event(
        &self,
        community_id: CommunityId,
        event: &nostr::Event,
        channel_id: Option<Uuid>,
    ) -> Result<(StoredEvent, bool)> {
        let result = event::insert_event(&self.pool, community_id, event, channel_id).await?;
        if result.1 {
            if let Err(e) = insert_mentions(&self.pool, community_id, event, channel_id).await {
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

    /// Return whether a creator-signed huddle-start event links a parent
    /// channel to an ephemeral huddle channel.
    pub async fn huddle_started_link_exists(
        &self,
        community_id: CommunityId,
        parent_channel_id: Uuid,
        ephemeral_channel_id: Uuid,
        creator_pubkey: &[u8],
    ) -> Result<bool> {
        event::huddle_started_link_exists(
            &self.pool,
            community_id,
            parent_channel_id,
            ephemeral_channel_id,
            creator_pubkey,
        )
        .await
    }

    /// Fetch the latest replaceable event for a (kind, pubkey) pair.
    ///
    /// Uses canonical NIP-16 ordering: `created_at DESC, id ASC`.
    /// This matches the write path in [`replace_addressable_event`] and handles
    /// historical duplicate survivors correctly.
    pub async fn get_latest_global_replaceable(
        &self,
        community_id: CommunityId,
        kind: i32,
        pubkey_bytes: &[u8],
    ) -> Result<Option<StoredEvent>> {
        event::get_latest_global_replaceable(&self.pool, community_id, kind, pubkey_bytes).await
    }

    /// Fetches a single non-deleted event by its raw ID bytes.
    ///
    /// Returns `None` if the event does not exist or has been soft-deleted.
    pub async fn get_event_by_id(
        &self,
        community_id: CommunityId,
        id_bytes: &[u8],
    ) -> Result<Option<StoredEvent>> {
        event::get_event_by_id(&self.pool, community_id, id_bytes).await
    }

    /// Fetches a single event by its raw ID bytes, **including soft-deleted rows**.
    pub async fn get_event_by_id_including_deleted(
        &self,
        community_id: CommunityId,
        id_bytes: &[u8],
    ) -> Result<Option<StoredEvent>> {
        event::get_event_by_id_including_deleted(&self.pool, community_id, id_bytes).await
    }

    /// Soft-deletes an event. Returns `Ok(true)` if deleted, `Ok(false)` if already deleted.
    pub async fn soft_delete_event(
        &self,
        community_id: CommunityId,
        event_id: &[u8],
    ) -> Result<bool> {
        event::soft_delete_event(&self.pool, community_id, event_id).await
    }

    /// Soft-delete the live row for an addressable coordinate `(kind, pubkey, d_tag)`.
    /// Used by NIP-09 a-tag deletion for parameterized-replaceable kinds.
    pub async fn soft_delete_by_coordinate(
        &self,
        community_id: CommunityId,
        kind: i32,
        pubkey: &[u8],
        d_tag: &str,
    ) -> Result<bool> {
        event::soft_delete_by_coordinate(&self.pool, community_id, kind, pubkey, d_tag).await
    }

    /// Atomically soft-delete an event and decrement thread reply counters.
    pub async fn soft_delete_event_and_update_thread(
        &self,
        community_id: CommunityId,
        event_id: &[u8],
        parent_event_id: Option<&[u8]>,
        root_event_id: Option<&[u8]>,
    ) -> Result<bool> {
        event::soft_delete_event_and_update_thread(
            &self.pool,
            community_id,
            event_id,
            parent_event_id,
            root_event_id,
        )
        .await
    }

    /// Returns the most recent `created_at` for a channel.
    pub async fn get_last_message_at(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
    ) -> Result<Option<DateTime<Utc>>> {
        event::get_last_message_at(&self.pool, community_id, channel_id).await
    }

    /// Bulk-fetch the most recent `created_at` for a set of channel IDs.
    pub async fn get_last_message_at_bulk(
        &self,
        community_id: CommunityId,
        channel_ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, DateTime<Utc>>> {
        event::get_last_message_at_bulk(&self.pool, community_id, channel_ids).await
    }

    /// Batch-fetch non-deleted events by their raw IDs.
    pub async fn get_events_by_ids(
        &self,
        community_id: CommunityId,
        ids: &[&[u8]],
    ) -> Result<Vec<StoredEvent>> {
        event::get_events_by_ids(&self.pool, community_id, ids).await
    }

    /// Atomically insert an event AND its thread metadata in a single transaction.
    pub async fn insert_event_with_thread_metadata(
        &self,
        community_id: CommunityId,
        event: &nostr::Event,
        channel_id: Option<Uuid>,
        thread_meta: Option<event::ThreadMetadataParams<'_>>,
    ) -> Result<(StoredEvent, bool)> {
        let result = event::insert_event_with_thread_metadata(
            &self.pool,
            community_id,
            event,
            channel_id,
            thread_meta,
        )
        .await?;
        if result.1 {
            if let Err(e) = insert_mentions(&self.pool, community_id, event, channel_id).await {
                tracing::warn!(event_id = %event.id, "Failed to insert mentions: {e}");
            }
        }
        Ok(result)
    }

    /// Atomically insert a kind:7 reaction event and its reaction row.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_reaction_event_with_thread_metadata(
        &self,
        community_id: CommunityId,
        event: &nostr::Event,
        channel_id: Option<Uuid>,
        thread_meta: Option<event::ThreadMetadataParams<'_>>,
        target_event_id: &[u8],
        actor_pubkey: &[u8],
        emoji: &str,
    ) -> Result<event::ReactionEventInsertOutcome> {
        let outcome = event::insert_reaction_event_with_thread_metadata(
            &self.pool,
            community_id,
            event,
            channel_id,
            thread_meta,
            target_event_id,
            actor_pubkey,
            emoji,
        )
        .await?;
        if let event::ReactionEventInsertOutcome::Inserted {
            was_inserted: true, ..
        } = &outcome
        {
            if let Err(e) = insert_mentions(&self.pool, community_id, event, channel_id).await {
                tracing::warn!(event_id = %event.id, "Failed to insert mentions: {e}");
            }
        }
        Ok(outcome)
    }

    /// Creates a new channel, bootstraps the creator as owner, and returns the record.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_channel(
        &self,
        community_id: CommunityId,
        name: &str,
        channel_type: channel::ChannelType,
        visibility: channel::ChannelVisibility,
        description: Option<&str>,
        created_by: &[u8],
        ttl_seconds: Option<i32>,
    ) -> Result<channel::ChannelRecord> {
        channel::create_channel(
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
    pub async fn create_channel_with_id(
        &self,
        community_id: CommunityId,
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
    pub async fn get_channel(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
    ) -> Result<channel::ChannelRecord> {
        channel::get_channel(&self.pool, community_id, channel_id).await
    }

    /// Returns the canvas content for a channel, if any.
    pub async fn get_canvas(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
    ) -> Result<Option<String>> {
        channel::get_canvas(&self.pool, community_id, channel_id).await
    }

    /// Sets or clears the canvas content for a channel.
    pub async fn set_canvas(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        canvas: Option<&str>,
    ) -> Result<()> {
        channel::set_canvas(&self.pool, community_id, channel_id, canvas).await
    }

    /// Adds a member to a channel.
    pub async fn add_member(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        pubkey: &[u8],
        role: channel::MemberRole,
        invited_by: Option<&[u8]>,
    ) -> Result<channel::MemberRecord> {
        channel::add_member(
            &self.pool,
            community_id,
            channel_id,
            pubkey,
            role,
            invited_by,
        )
        .await
    }

    /// Removes a member from a channel.
    pub async fn remove_member(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        pubkey: &[u8],
        actor_pubkey: &[u8],
    ) -> Result<()> {
        channel::remove_member(&self.pool, community_id, channel_id, pubkey, actor_pubkey).await
    }

    /// Returns `true` if the pubkey is an active member.
    pub async fn is_member(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        pubkey: &[u8],
    ) -> Result<bool> {
        channel::is_member(&self.pool, community_id, channel_id, pubkey).await
    }

    /// Returns all active members of a channel.
    pub async fn get_members(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
    ) -> Result<Vec<channel::MemberRecord>> {
        channel::get_members(&self.pool, community_id, channel_id).await
    }

    /// Returns active members for multiple channels in a single query.
    pub async fn get_members_bulk(
        &self,
        community_id: CommunityId,
        channel_ids: &[Uuid],
    ) -> Result<Vec<channel::MemberRecord>> {
        channel::get_members_bulk(&self.pool, community_id, channel_ids).await
    }

    /// Get all channel IDs accessible to a pubkey.
    pub async fn get_accessible_channel_ids(
        &self,
        community_id: CommunityId,
        pubkey: &[u8],
    ) -> Result<Vec<Uuid>> {
        channel::get_accessible_channel_ids(&self.pool, community_id, pubkey).await
    }

    /// Lists channels, optionally filtered by visibility.
    pub async fn list_channels(
        &self,
        community_id: CommunityId,
        visibility: Option<&str>,
    ) -> Result<Vec<channel::ChannelRecord>> {
        channel::list_channels(&self.pool, community_id, visibility).await
    }

    /// Returns full channel records for all channels a user can access.
    pub async fn get_accessible_channels(
        &self,
        community_id: CommunityId,
        pubkey: &[u8],
        visibility_filter: Option<&str>,
        member_only: Option<bool>,
    ) -> Result<Vec<channel::AccessibleChannel>> {
        channel::get_accessible_channels(
            &self.pool,
            community_id,
            pubkey,
            visibility_filter,
            member_only,
        )
        .await
    }

    /// Returns all bot-role members with their aggregated channel names in one community.
    pub async fn get_bot_members(
        &self,
        community_id: CommunityId,
    ) -> Result<Vec<channel::BotMemberRecord>> {
        channel::get_bot_members(&self.pool, community_id).await
    }

    /// Bulk-fetch user records by pubkey.
    pub async fn get_users_bulk(
        &self,
        community_id: CommunityId,
        pubkeys: &[Vec<u8>],
    ) -> Result<Vec<channel::UserRecord>> {
        channel::get_users_bulk(&self.pool, community_id, pubkeys).await
    }

    /// Updates a channel's name and/or description.
    pub async fn update_channel(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        updates: channel::ChannelUpdate,
    ) -> Result<channel::ChannelRecord> {
        channel::update_channel(&self.pool, community_id, channel_id, updates).await
    }

    /// Sets the topic for a channel.
    pub async fn set_topic(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        topic: &str,
        set_by: &[u8],
    ) -> Result<()> {
        channel::set_topic(&self.pool, community_id, channel_id, topic, set_by).await
    }

    /// Sets the purpose for a channel.
    pub async fn set_purpose(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        purpose: &str,
        set_by: &[u8],
    ) -> Result<()> {
        channel::set_purpose(&self.pool, community_id, channel_id, purpose, set_by).await
    }

    /// Archives a channel.
    pub async fn archive_channel(&self, community_id: CommunityId, channel_id: Uuid) -> Result<()> {
        channel::archive_channel(&self.pool, community_id, channel_id).await
    }

    /// Unarchives a channel.
    pub async fn unarchive_channel(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
    ) -> Result<()> {
        channel::unarchive_channel(&self.pool, community_id, channel_id).await
    }

    /// Soft-delete a channel.
    pub async fn soft_delete_channel(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
    ) -> Result<bool> {
        channel::soft_delete_channel(&self.pool, community_id, channel_id).await
    }

    /// Returns the count of active members in a channel.
    pub async fn get_member_count(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
    ) -> Result<i64> {
        channel::get_member_count(&self.pool, community_id, channel_id).await
    }

    /// Bulk-fetch member counts for a set of channel IDs.
    pub async fn get_member_counts_bulk(
        &self,
        community_id: CommunityId,
        channel_ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, i64>> {
        channel::get_member_counts_bulk(&self.pool, community_id, channel_ids).await
    }

    /// Get the active role of a pubkey in a channel.
    pub async fn get_member_role(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        pubkey: &[u8],
    ) -> Result<Option<String>> {
        channel::get_member_role(&self.pool, community_id, channel_id, pubkey).await
    }

    /// Bump the TTL deadline for an ephemeral channel after a new message.
    pub async fn bump_ttl_deadline(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
    ) -> Result<()> {
        channel::bump_ttl_deadline(&self.pool, community_id, channel_id).await
    }

    /// Archive ephemeral channels whose TTL deadline has passed.
    pub async fn reap_expired_ephemeral_channels(
        &self,
    ) -> Result<Vec<channel::ReapedEphemeralChannel>> {
        channel::reap_expired_ephemeral_channels(&self.pool).await
    }

    /// Query due reminders ready for delivery.
    pub async fn query_due_reminders(
        &self,
        now_secs: i64,
        batch_limit: i64,
    ) -> Result<Vec<event::DueReminder>> {
        event::query_due_reminders(&self.pool, now_secs, batch_limit).await
    }

    /// Atomically claim a due reminder for delivery (cross-pod dedup).
    pub async fn claim_due_reminder(
        &self,
        community_id: CommunityId,
        event_id: &[u8],
        event_created_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool> {
        event::claim_due_reminder(&self.pool, community_id, event_id, event_created_at).await
    }

    /// Atomically claim a due reminder using a caller-supplied delivery stamp.
    pub async fn claim_due_reminder_with_stamp(
        &self,
        community_id: CommunityId,
        event_id: &[u8],
        event_created_at: chrono::DateTime<chrono::Utc>,
        delivery_stamp: i64,
    ) -> Result<bool> {
        event::claim_due_reminder_with_stamp(
            &self.pool,
            community_id,
            event_id,
            event_created_at,
            delivery_stamp,
        )
        .await
    }

    /// Release a claimed due reminder after a publish failure.
    pub async fn release_due_reminder(
        &self,
        community_id: CommunityId,
        event_id: &[u8],
        event_created_at: chrono::DateTime<chrono::Utc>,
        delivery_stamp: i64,
    ) -> Result<bool> {
        event::release_due_reminder(
            &self.pool,
            community_id,
            event_id,
            event_created_at,
            delivery_stamp,
        )
        .await
    }

    /// Ensure a user record exists (upsert).
    pub async fn ensure_user(&self, community_id: CommunityId, pubkey: &[u8]) -> Result<()> {
        user::ensure_user(&self.pool, community_id, pubkey).await
    }

    /// Get a single user record by pubkey.
    pub async fn get_user(
        &self,
        community_id: CommunityId,
        pubkey: &[u8],
    ) -> Result<Option<user::UserProfile>> {
        user::get_user(&self.pool, community_id, pubkey).await
    }

    /// Update a user's profile fields.
    pub async fn update_user_profile(
        &self,
        community_id: CommunityId,
        pubkey: &[u8],
        display_name: Option<&str>,
        avatar_url: Option<&str>,
        about: Option<&str>,
        nip05_handle: Option<&str>,
    ) -> Result<()> {
        user::update_user_profile(
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
    pub async fn get_user_by_nip05(
        &self,
        community_id: CommunityId,
        local_part: &str,
        domain: &str,
    ) -> Result<Option<user::UserProfile>> {
        user::get_user_by_nip05(&self.pool, community_id, local_part, domain).await
    }

    /// Search users by display name, NIP-05 handle, or pubkey prefix.
    pub async fn search_users(
        &self,
        community_id: CommunityId,
        query: &str,
        limit: u32,
    ) -> Result<Vec<user::UserSearchProfile>> {
        user::search_users(&self.pool, community_id, query, limit).await
    }

    /// Atomically set agent owner — only if no owner is currently assigned.
    /// Returns Ok(true) if set, Ok(false) if an owner already exists.
    pub async fn set_agent_owner(
        &self,
        community_id: CommunityId,
        agent_pubkey: &[u8],
        owner_pubkey: &[u8],
    ) -> Result<bool> {
        user::set_agent_owner(&self.pool, community_id, agent_pubkey, owner_pubkey).await
    }

    /// Get the channel_add_policy and agent_owner_pubkey for a user.
    pub async fn get_agent_channel_policy(
        &self,
        community_id: CommunityId,
        pubkey: &[u8],
    ) -> Result<Option<(String, Option<Vec<u8>>)>> {
        user::get_agent_channel_policy(&self.pool, community_id, pubkey).await
    }

    /// Check whether `actor_pubkey` is the agent owner of `target_pubkey`.
    pub async fn is_agent_owner(
        &self,
        community_id: CommunityId,
        target_pubkey: &[u8],
        actor_pubkey: &[u8],
    ) -> Result<bool> {
        user::is_agent_owner(&self.pool, community_id, target_pubkey, actor_pubkey).await
    }

    /// Set the channel_add_policy for a user.
    pub async fn set_channel_add_policy(
        &self,
        community_id: CommunityId,
        pubkey: &[u8],
        policy: &str,
    ) -> Result<()> {
        user::set_channel_add_policy(&self.pool, community_id, pubkey, policy).await
    }

    /// Find an existing DM by its participant hash.
    pub async fn find_dm_by_participants(
        &self,
        community_id: CommunityId,
        participant_hash: &[u8],
    ) -> Result<Option<channel::ChannelRecord>> {
        dm::find_dm_by_participants(&self.pool, community_id, participant_hash).await
    }

    /// Create or return an existing DM channel.
    pub async fn create_dm(
        &self,
        community_id: CommunityId,
        participants: &[&[u8]],
        created_by: &[u8],
    ) -> Result<channel::ChannelRecord> {
        dm::create_dm(&self.pool, community_id, participants, created_by).await
    }

    /// List all DMs for a user.
    pub async fn list_dms_for_user(
        &self,
        community_id: CommunityId,
        pubkey: &[u8],
        limit: u32,
        cursor: Option<Uuid>,
    ) -> Result<Vec<dm::DmRecord>> {
        dm::list_dms_for_user(&self.pool, community_id, pubkey, limit, cursor).await
    }

    /// Open or retrieve a DM for the given participants.
    pub async fn open_dm(
        &self,
        community_id: CommunityId,
        pubkeys: &[&[u8]],
        created_by: &[u8],
    ) -> Result<(channel::ChannelRecord, bool)> {
        dm::open_dm(&self.pool, community_id, pubkeys, created_by).await
    }

    /// Hide a DM channel for a specific user.
    ///
    /// The DM is not deleted — it can be restored by opening a new DM with
    /// the same participants.
    pub async fn hide_dm(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        pubkey: &[u8],
    ) -> Result<()> {
        dm::hide_dm(&self.pool, community_id, channel_id, pubkey).await
    }

    /// Unhide a DM channel for a specific user.
    pub async fn unhide_dm(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        pubkey: &[u8],
    ) -> Result<()> {
        dm::unhide_dm(&self.pool, community_id, channel_id, pubkey).await
    }

    /// List the channel IDs of all DMs the given user currently has hidden.
    pub async fn list_hidden_dms(
        &self,
        community_id: CommunityId,
        pubkey: &[u8],
    ) -> Result<Vec<Uuid>> {
        dm::list_hidden_dms(&self.pool, community_id, pubkey).await
    }

    /// Insert thread metadata.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_thread_metadata(
        &self,
        community_id: CommunityId,
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
            community_id,
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
        community_id: CommunityId,
        root_event_id: &[u8],
        depth_limit: Option<u32>,
        limit: u32,
        cursor: Option<&[u8]>,
    ) -> Result<Vec<thread::ThreadReply>> {
        thread::get_thread_replies(
            &self.pool,
            community_id,
            root_event_id,
            depth_limit,
            limit,
            cursor,
        )
        .await
    }

    /// Fetch aggregated thread stats.
    pub async fn get_thread_summary(
        &self,
        community_id: CommunityId,
        event_id: &[u8],
    ) -> Result<Option<thread::ThreadSummary>> {
        thread::get_thread_summary(&self.pool, community_id, event_id).await
    }

    /// One channel window: top-level rows + summaries + server `has_more`.
    pub async fn get_channel_window(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        limit: u32,
        cursor: Option<(DateTime<Utc>, Vec<u8>)>,
        kind_filter: Option<&[u32]>,
    ) -> Result<thread::ChannelWindow> {
        thread::get_channel_window(
            &self.pool,
            community_id,
            channel_id,
            limit,
            cursor,
            kind_filter,
        )
        .await
    }

    /// Look up a single thread_metadata row by event_id.
    pub async fn get_thread_metadata_by_event(
        &self,
        community_id: CommunityId,
        event_id: &[u8],
    ) -> Result<Option<thread::ThreadMetadataRecord>> {
        thread::get_thread_metadata_by_event(&self.pool, community_id, event_id).await
    }

    /// Decrement reply counts.
    pub async fn decrement_reply_count(
        &self,
        community_id: CommunityId,
        parent_event_id: &[u8],
        root_event_id: Option<&[u8]>,
    ) -> Result<()> {
        thread::decrement_reply_count(&self.pool, community_id, parent_event_id, root_event_id)
            .await
    }

    /// Add (or re-activate) a reaction.
    pub async fn add_reaction(
        &self,
        community: CommunityId,
        event_id: &[u8],
        event_created_at: DateTime<Utc>,
        pubkey: &[u8],
        emoji: &str,
        reaction_event_id: Option<&[u8]>,
    ) -> Result<bool> {
        reaction::add_reaction(
            &self.pool,
            community,
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
        community: CommunityId,
        event_id: &[u8],
        event_created_at: DateTime<Utc>,
        pubkey: &[u8],
        emoji: &str,
    ) -> Result<bool> {
        reaction::remove_reaction(
            &self.pool,
            community,
            event_id,
            event_created_at,
            pubkey,
            emoji,
        )
        .await
    }

    /// Soft-delete a reaction by its source event ID.
    pub async fn remove_reaction_by_source_event_id(
        &self,
        community: CommunityId,
        reaction_event_id: &[u8],
    ) -> Result<bool> {
        reaction::remove_reaction_by_source_event_id(&self.pool, community, reaction_event_id).await
    }

    /// Look up the active reaction row for one actor + emoji + target tuple.
    pub async fn get_active_reaction_record(
        &self,
        community: CommunityId,
        event_id: &[u8],
        event_created_at: DateTime<Utc>,
        pubkey: &[u8],
        emoji: &str,
    ) -> Result<Option<reaction::ActiveReactionRecord>> {
        reaction::get_active_reaction_record(
            &self.pool,
            community,
            event_id,
            event_created_at,
            pubkey,
            emoji,
        )
        .await
    }

    /// Backfill the source event ID on an active reaction row.
    pub async fn set_reaction_event_id(
        &self,
        community: CommunityId,
        event_id: &[u8],
        event_created_at: DateTime<Utc>,
        pubkey: &[u8],
        emoji: &str,
        reaction_event_id: &[u8],
    ) -> Result<bool> {
        reaction::set_reaction_event_id(
            &self.pool,
            community,
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
        community: CommunityId,
        event_id: &[u8],
        event_created_at: DateTime<Utc>,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<Vec<reaction::ReactionGroup>> {
        reaction::get_reactions(
            &self.pool,
            community,
            event_id,
            event_created_at,
            limit,
            cursor,
        )
        .await
    }

    /// Batch-fetch emoji counts for a set of (event_id, event_created_at) pairs.
    pub async fn get_reactions_bulk(
        &self,
        community: CommunityId,
        event_ids: &[(&[u8], DateTime<Utc>)],
    ) -> Result<Vec<reaction::BulkReactionEntry>> {
        reaction::get_reactions_bulk(&self.pool, community, event_ids).await
    }

    /// Find events that @mention the given pubkey.
    pub async fn query_feed_mentions(
        &self,
        community: CommunityId,
        pubkey_bytes: &[u8],
        accessible_channel_ids: &[Uuid],
        since: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<StoredEvent>> {
        feed::query_mentions(
            &self.pool,
            community,
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
        community: CommunityId,
        pubkey_bytes: &[u8],
        accessible_channel_ids: &[Uuid],
        since: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<StoredEvent>> {
        feed::query_needs_action(
            &self.pool,
            community,
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
        community: CommunityId,
        accessible_channel_ids: &[Uuid],
        since: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<StoredEvent>> {
        feed::query_activity(&self.pool, community, accessible_channel_ids, since, limit).await
    }

    /// Create a new API token record.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_api_token(
        &self,
        community_id: CommunityId,
        token_hash: &[u8],
        owner_pubkey: &[u8],
        name: &str,
        scopes: &[String],
        channel_ids: Option<&[Uuid]>,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<Uuid> {
        api_token::create_api_token(
            &self.pool,
            *community_id.as_uuid(),
            token_hash,
            owner_pubkey,
            name,
            scopes,
            channel_ids,
            expires_at,
        )
        .await
    }

    /// Atomic conditional INSERT with 10-token limit (per (community, owner)).
    #[allow(clippy::too_many_arguments)]
    pub async fn create_api_token_if_under_limit(
        &self,
        community_id: CommunityId,
        token_hash: &[u8],
        owner_pubkey: &[u8],
        name: &str,
        scopes: &[String],
        channel_ids: Option<&[Uuid]>,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<Option<Uuid>> {
        api_token::create_api_token_if_under_limit(
            &self.pool,
            *community_id.as_uuid(),
            token_hash,
            owner_pubkey,
            name,
            scopes,
            channel_ids,
            expires_at,
        )
        .await
    }

    /// Look up an active (non-revoked) API token by its SHA-256 hash,
    /// scoped to the request's community.
    ///
    /// See [`api_token::get_api_token_by_hash_including_revoked`] for the
    /// row-44 conformance rationale — the `(community_id, token_hash)` key
    /// is enforced both by the storage UNIQUE index and by this WHERE clause.
    pub async fn get_api_token_by_hash(
        &self,
        community_id: CommunityId,
        hash: &[u8],
    ) -> Result<Option<ApiTokenRecord>> {
        let row = sqlx::query(
            r#"
            SELECT id, token_hash, owner_pubkey, name, scopes, channel_ids,
                   created_at, expires_at, last_used_at, revoked_at
            FROM api_tokens
            WHERE community_id = $1 AND token_hash = $2 AND revoked_at IS NULL
            "#,
        )
        .bind(community_id.as_uuid())
        .bind(hash)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            None => Ok(None),
            Some(r) => parse_api_token_row(r).map(Some),
        }
    }

    /// Look up an API token by hash, including revoked, scoped to community.
    pub async fn get_api_token_by_hash_including_revoked(
        &self,
        community_id: CommunityId,
        hash: &[u8],
    ) -> Result<Option<ApiTokenRecord>> {
        api_token::get_api_token_by_hash_including_revoked(
            &self.pool,
            *community_id.as_uuid(),
            hash,
        )
        .await
    }

    /// Record a token usage (update `last_used_at`), scoped to community.
    pub async fn touch_api_token(&self, community_id: CommunityId, hash: &[u8]) -> Result<()> {
        sqlx::query(
            "UPDATE api_tokens SET last_used_at = NOW() WHERE community_id = $1 AND token_hash = $2",
        )
        .bind(community_id.as_uuid())
        .bind(hash)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Alias for [`Self::touch_api_token`].
    pub async fn update_token_last_used(
        &self,
        community_id: CommunityId,
        hash: &[u8],
    ) -> Result<()> {
        self.touch_api_token(community_id, hash).await
    }

    /// List all active (non-revoked) tokens in a community, newest first.
    pub async fn list_active_tokens(&self, community_id: CommunityId) -> Result<Vec<TokenSummary>> {
        let rows = sqlx::query(
            r#"
            SELECT id, name, owner_pubkey, scopes, created_at, expires_at
            FROM api_tokens
            WHERE community_id = $1 AND revoked_at IS NULL
            ORDER BY created_at DESC
            LIMIT 1000
            "#,
        )
        .bind(community_id.as_uuid())
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

    /// List all tokens for a (community, owner) pair (including revoked).
    pub async fn list_tokens_by_owner(
        &self,
        community_id: CommunityId,
        pubkey: &[u8],
    ) -> Result<Vec<ApiTokenRecord>> {
        api_token::list_tokens_by_owner(&self.pool, *community_id.as_uuid(), pubkey).await
    }

    /// Revoke a single token by ID, scoped to (community, owner).
    pub async fn revoke_token(
        &self,
        community_id: CommunityId,
        id: Uuid,
        owner_pubkey: &[u8],
        revoked_by: &[u8],
    ) -> Result<bool> {
        api_token::revoke_token(
            &self.pool,
            *community_id.as_uuid(),
            id,
            owner_pubkey,
            revoked_by,
        )
        .await
    }

    /// Revoke all active tokens for a (community, owner) pair.
    pub async fn revoke_all_tokens(
        &self,
        community_id: CommunityId,
        owner_pubkey: &[u8],
        revoked_by: &[u8],
    ) -> Result<u64> {
        api_token::revoke_all_tokens(
            &self.pool,
            *community_id.as_uuid(),
            owner_pubkey,
            revoked_by,
        )
        .await
    }

    /// Create a new workflow.
    pub async fn create_workflow(
        &self,
        community_id: CommunityId,
        channel_id: Option<Uuid>,
        owner_pubkey: &[u8],
        name: &str,
        definition_json: &str,
        definition_hash: &[u8],
    ) -> Result<Uuid> {
        workflow::create_workflow(
            &self.pool,
            community_id,
            channel_id,
            owner_pubkey,
            name,
            definition_json,
            definition_hash,
        )
        .await
    }

    /// Insert or update a workflow using its NIP-33 `d`-tag UUID.
    #[allow(clippy::too_many_arguments)]
    pub async fn upsert_workflow(
        &self,
        community_id: CommunityId,
        id: Uuid,
        channel_id: Option<Uuid>,
        owner_pubkey: &[u8],
        name: &str,
        definition_json: &str,
        definition_hash: &[u8],
    ) -> Result<()> {
        workflow::upsert_workflow(
            &self.pool,
            community_id,
            id,
            channel_id,
            owner_pubkey,
            name,
            definition_json,
            definition_hash,
        )
        .await
    }

    /// Fetch a single workflow by ID, scoped to its community.
    pub async fn get_workflow(
        &self,
        community_id: CommunityId,
        id: Uuid,
    ) -> Result<workflow::WorkflowRecord> {
        workflow::get_workflow(&self.pool, community_id, id).await
    }

    /// List workflows for a channel.
    pub async fn list_channel_workflows(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Vec<workflow::WorkflowRecord>> {
        workflow::list_channel_workflows(&self.pool, community_id, channel_id, limit, offset).await
    }

    /// List active, enabled workflows for a channel.
    pub async fn list_enabled_channel_workflows(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
    ) -> Result<Vec<workflow::WorkflowRecord>> {
        workflow::list_enabled_channel_workflows(&self.pool, community_id, channel_id).await
    }

    /// List all active, enabled schedule-triggered workflows.
    pub async fn list_all_enabled_workflows(&self) -> Result<Vec<workflow::WorkflowRecord>> {
        workflow::list_all_enabled_workflows(&self.pool).await
    }

    /// Claim a scheduled workflow fire for an authoritative schedule instant.
    ///
    /// Returns `Some` only for the first pod to claim `(community_id,
    /// workflow_id, scheduled_for)`; all other pods must skip creating a run.
    /// `community_id` is server provenance (the workflow row's own community
    /// from the scheduler scan), never client-supplied — `workflows` is keyed
    /// `(community_id, id)`, so the claim must bind both to avoid fanning
    /// across communities that share the workflow UUID.
    pub async fn claim_scheduled_workflow_fire(
        &self,
        community_id: CommunityId,
        workflow_id: Uuid,
        scheduled_for: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<workflow::ScheduledWorkflowFireClaim>> {
        workflow::claim_scheduled_workflow_fire(
            &self.pool,
            community_id,
            workflow_id,
            scheduled_for,
        )
        .await
    }

    /// Fetch the latest claimed schedule instant for interval trigger anchoring.
    pub async fn latest_scheduled_workflow_fire(
        &self,
        community_id: CommunityId,
        workflow_id: Uuid,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>> {
        workflow::latest_scheduled_workflow_fire(&self.pool, community_id, workflow_id).await
    }

    /// Attach the workflow run id created from a won scheduled-fire claim.
    pub async fn attach_scheduled_workflow_run(
        &self,
        community_id: CommunityId,
        workflow_id: Uuid,
        scheduled_for: chrono::DateTime<chrono::Utc>,
        workflow_run_id: Uuid,
    ) -> Result<bool> {
        workflow::attach_scheduled_workflow_run(
            &self.pool,
            community_id,
            workflow_id,
            scheduled_for,
            workflow_run_id,
        )
        .await
    }

    /// Delete old scheduled workflow fire claims before a retention cutoff.
    pub async fn prune_scheduled_workflow_fires_before(
        &self,
        older_than: chrono::DateTime<chrono::Utc>,
    ) -> Result<u64> {
        workflow::prune_scheduled_workflow_fires_before(&self.pool, older_than).await
    }

    /// Update a workflow's name, definition, and hash.
    pub async fn update_workflow(
        &self,
        community_id: CommunityId,
        id: Uuid,
        name: &str,
        definition_json: &str,
        definition_hash: &[u8],
    ) -> Result<()> {
        workflow::update_workflow(
            &self.pool,
            community_id,
            id,
            name,
            definition_json,
            definition_hash,
        )
        .await
    }

    /// Update a workflow's status.
    pub async fn update_workflow_status(
        &self,
        community_id: CommunityId,
        id: Uuid,
        status: workflow::WorkflowStatus,
    ) -> Result<()> {
        workflow::update_workflow_status(&self.pool, community_id, id, status).await
    }

    /// Enable or disable a workflow.
    pub async fn set_workflow_enabled(
        &self,
        community_id: CommunityId,
        id: Uuid,
        enabled: bool,
    ) -> Result<()> {
        workflow::set_workflow_enabled(&self.pool, community_id, id, enabled).await
    }

    /// Delete a workflow and all its runs/approvals.
    pub async fn delete_workflow(&self, community_id: CommunityId, id: Uuid) -> Result<()> {
        workflow::delete_workflow(&self.pool, community_id, id).await
    }

    /// Delete a workflow only when it belongs to the provided owner.
    /// Returns the deleted workflow's `channel_id`.
    pub async fn delete_workflow_for_owner(
        &self,
        community_id: CommunityId,
        id: Uuid,
        owner_pubkey: &[u8],
    ) -> Result<Option<Uuid>> {
        workflow::delete_workflow_for_owner(&self.pool, community_id, id, owner_pubkey).await
    }

    /// Find a workflow by owner pubkey and name within a community. Used for
    /// NIP-09 a-tag deletion where the d-tag is the workflow name (not UUID).
    pub async fn find_workflow_by_owner_and_name(
        &self,
        community_id: CommunityId,
        owner_pubkey: &[u8],
        name: &str,
    ) -> Result<Option<workflow::WorkflowRecord>> {
        workflow::find_by_owner_and_name(&self.pool, community_id, owner_pubkey, name).await
    }

    /// Create a new workflow run.
    pub async fn create_workflow_run(
        &self,
        community_id: CommunityId,
        workflow_id: Uuid,
        trigger_event_id: Option<&[u8]>,
        trigger_context: Option<&serde_json::Value>,
    ) -> Result<Uuid> {
        workflow::create_workflow_run(
            &self.pool,
            community_id,
            workflow_id,
            trigger_event_id,
            trigger_context,
        )
        .await
    }

    /// Fetch a single workflow run, scoped to its community.
    pub async fn get_workflow_run(
        &self,
        community_id: CommunityId,
        id: Uuid,
    ) -> Result<workflow::WorkflowRunRecord> {
        workflow::get_workflow_run(&self.pool, community_id, id).await
    }

    /// List runs for a workflow.
    pub async fn list_workflow_runs(
        &self,
        community_id: CommunityId,
        workflow_id: Uuid,
        limit: i64,
    ) -> Result<Vec<workflow::WorkflowRunRecord>> {
        workflow::list_workflow_runs(&self.pool, community_id, workflow_id, limit).await
    }

    /// Update a workflow run's status.
    pub async fn update_workflow_run(
        &self,
        community_id: CommunityId,
        id: Uuid,
        status: workflow::RunStatus,
        current_step: i32,
        trace: &serde_json::Value,
        error: Option<&str>,
    ) -> Result<()> {
        workflow::update_workflow_run(
            &self.pool,
            community_id,
            id,
            status,
            current_step,
            trace,
            error,
        )
        .await
    }

    /// Create an approval request.
    pub async fn create_approval(&self, params: workflow::CreateApprovalParams<'_>) -> Result<()> {
        workflow::create_approval(&self.pool, params).await
    }

    /// Fetch an approval by raw token.
    pub async fn get_approval(
        &self,
        community_id: CommunityId,
        token: &str,
    ) -> Result<workflow::ApprovalRecord> {
        workflow::get_approval(&self.pool, community_id, token).await
    }

    /// Fetch an approval by its already-hashed token (no re-hashing).
    pub async fn get_approval_by_stored_hash(
        &self,
        community_id: CommunityId,
        token_hash: &[u8],
    ) -> Result<workflow::ApprovalRecord> {
        workflow::get_approval_by_stored_hash(&self.pool, community_id, token_hash).await
    }

    /// Fetch all approvals for a workflow run.
    pub async fn get_run_approvals(
        &self,
        community_id: CommunityId,
        workflow_id: uuid::Uuid,
        run_id: uuid::Uuid,
    ) -> Result<Vec<workflow::ApprovalRecord>> {
        workflow::get_run_approvals(&self.pool, community_id, workflow_id, run_id).await
    }

    /// Update an approval's status.
    pub async fn update_approval(
        &self,
        community_id: CommunityId,
        token: &str,
        status: workflow::ApprovalStatus,
        approver_pubkey: Option<&[u8]>,
        note: Option<&str>,
    ) -> Result<bool> {
        workflow::update_approval(
            &self.pool,
            community_id,
            token,
            status,
            approver_pubkey,
            note,
        )
        .await
    }

    /// Update an approval by its already-hashed token (no re-hashing).
    pub async fn update_approval_by_stored_hash(
        &self,
        community_id: CommunityId,
        token_hash: &[u8],
        status: workflow::ApprovalStatus,
        approver_pubkey: Option<&[u8]>,
        note: Option<&str>,
    ) -> Result<bool> {
        workflow::update_approval_by_stored_hash(
            &self.pool,
            community_id,
            token_hash,
            status,
            approver_pubkey,
            note,
        )
        .await
    }

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

    /// Check if a pubkey is in the allowlist for `community`.
    pub async fn is_pubkey_allowed(&self, community: CommunityId, pubkey: &[u8]) -> Result<bool> {
        let row = sqlx::query(
            "SELECT COUNT(*) as cnt FROM pubkey_allowlist WHERE community_id = $1 AND pubkey = $2",
        )
        .bind(community.as_uuid())
        .bind(pubkey)
        .fetch_one(&self.pool)
        .await?;
        let cnt: i64 = row.try_get("cnt")?;
        Ok(cnt > 0)
    }

    /// Check if the community allowlist has any entries (i.e. is enforcement active).
    pub async fn has_allowlist_entries(&self, community: CommunityId) -> Result<bool> {
        let row =
            sqlx::query("SELECT COUNT(*) as cnt FROM pubkey_allowlist WHERE community_id = $1")
                .bind(community.as_uuid())
                .fetch_one(&self.pool)
                .await?;
        let cnt: i64 = row.try_get("cnt")?;
        Ok(cnt > 0)
    }

    /// Add a pubkey to the community allowlist.
    pub async fn add_to_allowlist(
        &self,
        community: CommunityId,
        pubkey: &[u8],
        added_by: &[u8],
        note: Option<&str>,
    ) -> Result<bool> {
        let result = sqlx::query(
            "INSERT INTO pubkey_allowlist (community_id, pubkey, added_by, note) VALUES ($1, $2, $3, $4) \
             ON CONFLICT DO NOTHING",
        )
        .bind(community.as_uuid())
        .bind(pubkey)
        .bind(added_by)
        .bind(note)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Remove a pubkey from the community allowlist.
    pub async fn remove_from_allowlist(
        &self,
        community: CommunityId,
        pubkey: &[u8],
    ) -> Result<bool> {
        let result =
            sqlx::query("DELETE FROM pubkey_allowlist WHERE community_id = $1 AND pubkey = $2")
                .bind(community.as_uuid())
                .bind(pubkey)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }

    /// List all pubkeys in the community allowlist.
    pub async fn list_allowlist(&self, community: CommunityId) -> Result<Vec<AllowlistEntry>> {
        let rows = sqlx::query(
            "SELECT pubkey, added_by, added_at, note FROM pubkey_allowlist WHERE community_id = $1 ORDER BY added_at DESC",
        )
        .bind(community.as_uuid())
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

    /// Returns `true` if `pubkey` (64-char hex) is a member of `community`.
    pub async fn is_relay_member(&self, community: CommunityId, pubkey: &str) -> Result<bool> {
        relay_members::is_relay_member(&self.pool, community, pubkey).await
    }

    /// Returns the relay member record for `pubkey` in `community`, or `None` if not found.
    pub async fn get_relay_member(
        &self,
        community: CommunityId,
        pubkey: &str,
    ) -> Result<Option<relay_members::RelayMember>> {
        relay_members::get_relay_member(&self.pool, community, pubkey).await
    }

    /// Returns all relay members of `community` ordered by `created_at` ascending.
    pub async fn list_relay_members(
        &self,
        community: CommunityId,
    ) -> Result<Vec<relay_members::RelayMember>> {
        relay_members::list_relay_members(&self.pool, community).await
    }

    /// Adds a new relay member to `community`.
    ///
    /// Returns `true` if the row was actually inserted, `false` if the pubkey
    /// already existed in `community` (idempotent — `ON CONFLICT DO NOTHING`).
    pub async fn add_relay_member(
        &self,
        community: CommunityId,
        pubkey: &str,
        role: &str,
        added_by: Option<&str>,
    ) -> Result<bool> {
        relay_members::add_relay_member(&self.pool, community, pubkey, role, added_by).await
    }

    /// Removes a relay member from `community` atomically, refusing to delete the owner.
    pub async fn remove_relay_member(
        &self,
        community: CommunityId,
        pubkey: &str,
    ) -> Result<relay_members::RemoveResult> {
        relay_members::remove_relay_member(&self.pool, community, pubkey).await
    }

    /// Removes a relay member from `community` only if their current role matches `expected_role`.
    ///
    /// Atomic conditional delete — eliminates the TOCTOU race between a
    /// prior role read and the delete. See [`relay_members::remove_relay_member_if_role`].
    pub async fn remove_relay_member_if_role(
        &self,
        community: CommunityId,
        pubkey: &str,
        expected_role: &str,
    ) -> Result<relay_members::RemoveResult> {
        relay_members::remove_relay_member_if_role(&self.pool, community, pubkey, expected_role)
            .await
    }

    /// Updates the role of an existing relay member in `community`. Returns `true` if updated.
    pub async fn update_relay_member_role(
        &self,
        community: CommunityId,
        pubkey: &str,
        new_role: &str,
    ) -> Result<bool> {
        relay_members::update_relay_member_role(&self.pool, community, pubkey, new_role).await
    }

    /// Ensures the owner pubkey exists with role `"owner"` in `community`. Called at startup.
    pub async fn bootstrap_owner(&self, community: CommunityId, owner_pubkey: &str) -> Result<()> {
        relay_members::bootstrap_owner(&self.pool, community, owner_pubkey).await
    }

    /// Migrates existing `pubkey_allowlist` entries into `relay_members` for `community`.
    ///
    /// Idempotent — uses `ON CONFLICT DO NOTHING`. Returns the number of rows
    /// inserted, or 0 if the `pubkey_allowlist` table doesn't exist.
    pub async fn backfill_from_allowlist(&self, community: CommunityId) -> Result<u64> {
        relay_members::backfill_from_allowlist(&self.pool, community).await
    }

    /// Return the current owner of git repo name `repo_id` in `community`, or
    /// `None` if unreserved. See [`git_repo::repo_name_owner`].
    pub async fn repo_name_owner(
        &self,
        community: CommunityId,
        repo_id: &str,
    ) -> Result<Option<String>> {
        git_repo::repo_name_owner(&self.pool, community, repo_id).await
    }

    /// Reserve a git repo name for `owner_pubkey` in `community` (NIP-34).
    ///
    /// See [`git_repo::reserve_repo_name`] for the outcome semantics. The
    /// per-pubkey quota is enforced by the caller against `count_repos_for_owner`.
    pub async fn reserve_repo_name(
        &self,
        community: CommunityId,
        repo_id: &str,
        owner_pubkey: &str,
    ) -> Result<git_repo::ReserveOutcome> {
        git_repo::reserve_repo_name(&self.pool, community, repo_id, owner_pubkey).await
    }

    /// Count git repos reserved by `owner_pubkey` in `community` (quota check).
    pub async fn count_repos_for_owner(
        &self,
        community: CommunityId,
        owner_pubkey: &str,
    ) -> Result<i64> {
        git_repo::count_repos_for_owner(&self.pool, community, owner_pubkey).await
    }

    /// Release a git repo name reservation held by `owner_pubkey` (rollback).
    ///
    /// Returns the number of rows removed (0 or 1). See [`git_repo::release_repo_name`].
    pub async fn release_repo_name(
        &self,
        community: CommunityId,
        repo_id: &str,
        owner_pubkey: &str,
    ) -> Result<u64> {
        git_repo::release_repo_name(&self.pool, community, repo_id, owner_pubkey).await
    }

    /// Returns `true` if `pubkey` (64-char hex) is archived in `community_id`.
    pub async fn is_archived(&self, community_id: CommunityId, pubkey: &str) -> Result<bool> {
        archived_identities::is_archived(&self.pool, community_id, pubkey).await
    }

    /// Archives an identity in `community_id`. Returns `true` if inserted, `false` if already archived.
    #[allow(clippy::too_many_arguments)]
    pub async fn archive(
        &self,
        community_id: CommunityId,
        pubkey: &str,
        consent_path: &str,
        actor: &str,
        reason: Option<&str>,
        replaced_by: Option<&str>,
        request_event_id: &str,
    ) -> Result<bool> {
        archived_identities::archive(
            &self.pool,
            community_id,
            pubkey,
            consent_path,
            actor,
            reason,
            replaced_by,
            request_event_id,
        )
        .await
    }

    /// Unarchives an identity from `community_id`. Returns `true` if deleted, `false` if absent.
    pub async fn unarchive(&self, community_id: CommunityId, pubkey: &str) -> Result<bool> {
        archived_identities::unarchive(&self.pool, community_id, pubkey).await
    }

    /// Returns all identities archived in `community_id`, ordered by archive time ascending.
    pub async fn list_archived(
        &self,
        community_id: CommunityId,
    ) -> Result<Vec<archived_identities::ArchivedIdentity>> {
        archived_identities::list_archived(&self.pool, community_id).await
    }

    /// Soft-delete NIP-29 discovery events for a channel created by a specific relay pubkey.
    pub async fn soft_delete_discovery_events(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        relay_pubkey: &[u8],
    ) -> Result<u64> {
        let result = sqlx::query(
            "UPDATE events SET deleted_at = NOW() \
             WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3 AND deleted_at IS NULL AND kind IN (39000, 39001, 39002)",
        )
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .bind(relay_pubkey)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Atomically replace a replaceable event: NIP-16 kinds (0, 3, 41, 10000–19999)
    /// and NIP-29 discovery state (39000–39002, called from side_effects.rs).
    ///
    /// Keeps only the event with the highest `created_at` per (kind, pubkey, channel_id).
    /// Same-second ties are broken by lowest event `id` (NIP-16 deterministic ordering).
    /// Returns `(event, false)` for stale writes and duplicate IDs — callers should
    /// skip fan-out/dispatch when `was_inserted` is false.
    pub async fn replace_addressable_event(
        &self,
        community_id: CommunityId,
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
            for b in community_id.as_uuid().as_bytes() {
                h ^= *b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
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
             WHERE community_id = $1 AND kind = $2 AND pubkey = $3 \
             AND channel_id IS NOT DISTINCT FROM $4 \
             AND deleted_at IS NULL \
             ORDER BY created_at DESC, id ASC LIMIT 1",
        )
        .bind(community_id.as_uuid())
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
             WHERE community_id = $1 AND kind = $2 AND pubkey = $3 \
             AND channel_id IS NOT DISTINCT FROM $4 \
             AND deleted_at IS NULL",
        )
        .bind(community_id.as_uuid())
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
            "INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id, d_tag) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
             ON CONFLICT DO NOTHING",
        )
        .bind(community_id.as_uuid())
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
        if let Err(e) = crate::insert_mentions(&self.pool, community_id, event, channel_id).await {
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
        community_id: CommunityId,
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
            for b in community_id.as_uuid().as_bytes() {
                h ^= *b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
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
             WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND d_tag = $4 AND deleted_at IS NULL \
             ORDER BY created_at DESC, id ASC LIMIT 1",
        )
        .bind(community_id.as_uuid())
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
                 WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND d_tag = $4 AND deleted_at IS NULL",
            )
            .bind(community_id.as_uuid())
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
            "INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id, d_tag, not_before) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) \
             ON CONFLICT DO NOTHING",
        )
        .bind(community_id.as_uuid())
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
        .bind(event::extract_not_before(event))
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
        if let Err(e) = crate::insert_mentions(&self.pool, community_id, event, channel_id).await {
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

#[cfg(test)]
mod tests {
    //! Pin the load-bearing contract for `Db::communities_of_channels`:
    //! a channel id that does NOT exist MUST be absent from the result
    //! map, never mapped to a default. The relay-side read-row emitter
    //! relies on this — a missing entry triggers `MissingLookup →
    //! ImplBug{row_community_lookup_missing} → CoverageBreach`. If this
    //! helper ever started returning a default/zero entry for unknown
    //! channels, that fail-closed chain would go blind.
    use super::*;
    use buzz_core::CommunityId;
    use sqlx::PgPool;
    use uuid::Uuid;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

    async fn setup_db() -> Db {
        let pool = PgPool::connect(TEST_DB_URL)
            .await
            .expect("connect to test DB");
        Db::from_pool(pool)
    }

    async fn make_community(pool: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        let host = format!("communities-of-channels-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(host)
            .execute(pool)
            .await
            .expect("insert community");
        id
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn lookup_community_by_host_matches_case_insensitive_host_index() {
        let db = setup_db().await;
        let id = Uuid::new_v4();
        let lower_host = format!("lookup-community-{}.example", id.simple());
        let stored_host = lower_host.to_uppercase();

        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(&stored_host)
            .execute(&db.pool)
            .await
            .expect("insert mixed-case community host");

        let found = db
            .lookup_community_by_host(&lower_host)
            .await
            .expect("lookup lower-case host")
            .expect("community found by lower-case host");
        assert_eq!(found.id, CommunityId::from_uuid(id));
        assert_eq!(found.host, stored_host);

        let found = db
            .lookup_community_by_host(&stored_host)
            .await
            .expect("lookup stored-case host")
            .expect("community found by stored-case host");
        assert_eq!(found.id, CommunityId::from_uuid(id));
    }

    async fn insert_channel(pool: &PgPool, community_id: Uuid, channel_id: Uuid) {
        let creator: Vec<u8> = vec![0u8; 32];
        sqlx::query(
            r#"
            INSERT INTO channels
                (id, community_id, name, channel_type, visibility, created_by)
            VALUES
                ($1, $2, $3, 'stream'::channel_type, 'open'::channel_visibility, $4)
            "#,
        )
        .bind(channel_id)
        .bind(community_id)
        .bind(format!("ch-{}", channel_id.simple()))
        .bind(&creator)
        .execute(pool)
        .await
        .expect("insert channel");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn allowlist_is_scoped_to_community() {
        let db = setup_db().await;
        let community_a = CommunityId::from_uuid(make_community(&db.pool).await);
        let community_b = CommunityId::from_uuid(make_community(&db.pool).await);
        let pubkey = [7u8; 32];
        let added_by = [9u8; 32];

        assert!(db
            .add_to_allowlist(community_a, &pubkey, &added_by, Some("a-only"))
            .await
            .expect("add allowlist row"));
        assert!(!db
            .add_to_allowlist(community_a, &pubkey, &added_by, Some("duplicate"))
            .await
            .expect("duplicate allowlist row is idempotent"));

        assert!(
            db.is_pubkey_allowed(community_a, &pubkey)
                .await
                .expect("allowlist check A"),
            "pubkey added to A must be allowed in A"
        );
        assert!(
            !db.is_pubkey_allowed(community_b, &pubkey)
                .await
                .expect("allowlist check B"),
            "pubkey added only to A must not be allowed in B"
        );
        assert!(db
            .has_allowlist_entries(community_a)
            .await
            .expect("A has entries"));
        assert!(!db
            .has_allowlist_entries(community_b)
            .await
            .expect("B has no entries"));

        let listed = db
            .list_allowlist(community_a)
            .await
            .expect("list A allowlist");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].pubkey, pubkey);

        assert!(
            !db.remove_from_allowlist(community_b, &pubkey)
                .await
                .expect("remove from B is no-op"),
            "removing from B must not delete A's row"
        );
        assert!(db
            .is_pubkey_allowed(community_a, &pubkey)
            .await
            .expect("A still allowed after B remove"));
        assert!(db
            .remove_from_allowlist(community_a, &pubkey)
            .await
            .expect("remove from A"));
        assert!(!db
            .is_pubkey_allowed(community_a, &pubkey)
            .await
            .expect("A not allowed after remove"));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn communities_of_channels_present_for_existing_absent_for_missing() {
        let db = setup_db().await;
        let community = make_community(&db.pool).await;
        let existing = Uuid::new_v4();
        insert_channel(&db.pool, community, existing).await;

        // Channel that is NOT inserted — the load-bearing case.
        let missing = Uuid::new_v4();

        let result = db
            .communities_of_channels(&[existing, missing])
            .await
            .expect("communities_of_channels");

        // (1) Existing channel → present with its true community.
        assert_eq!(
            result.get(&existing).copied(),
            Some(CommunityId::from_uuid(community)),
            "existing channel must map to its true community",
        );

        // (2) Missing channel → ABSENT from the map (never defaulted).
        // This is the contract the relay-side `MissingLookup → ImplBug`
        // fail-closed guard-rail depends on. If this assertion ever
        // weakens to `result.get(&missing) != Some(community)`, the
        // mutate-bite below stops biting.
        assert!(
            !result.contains_key(&missing),
            "missing channel must be absent from the result map, got {:?}",
            result.get(&missing),
        );

        // (3) Map size matches: exactly one entry, the existing one.
        assert_eq!(
            result.len(),
            1,
            "result map must contain only existing channels"
        );
    }

    /// BUG-5 regression: the `reactions` table is community-scoped
    /// (`PK (community_id, event_created_at, event_id, pubkey, emoji)`), so a
    /// reaction added under community A must be invisible and unremovable from
    /// community B — even for the *identical* `(event_id, pubkey, emoji)` shape.
    /// Before the fix, `add_reaction` omitted `community_id` (NOT NULL → 500) and
    /// every read/remove filtered `event_id` only (latent cross-tenant bleed).
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn reactions_are_scoped_to_community() {
        let db = setup_db().await;
        let community_a = CommunityId::from_uuid(make_community(&db.pool).await);
        let community_b = CommunityId::from_uuid(make_community(&db.pool).await);

        // Identical referenced-event shape across both tenants.
        let event_id = [0xABu8; 32];
        let event_created_at = Utc::now();
        let pubkey = [7u8; 32];
        let emoji = "👍";

        // (1) Add succeeds under A (this INSERT 500'd before the fix).
        assert!(
            db.add_reaction(
                community_a,
                &event_id,
                event_created_at,
                &pubkey,
                emoji,
                None
            )
            .await
            .expect("add reaction under A"),
            "first reaction under A must be inserted"
        );
        // Idempotent: re-adding the same active reaction is a no-op.
        assert!(
            !db.add_reaction(
                community_a,
                &event_id,
                event_created_at,
                &pubkey,
                emoji,
                None
            )
            .await
            .expect("duplicate reaction under A"),
            "active duplicate under A must not re-insert"
        );

        // (2) Visible on A, invisible on B (grouped read path).
        let groups_a = db
            .get_reactions(community_a, &event_id, event_created_at, 100, None)
            .await
            .expect("get reactions A");
        assert_eq!(groups_a.len(), 1, "A must see its own reaction group");
        assert_eq!(groups_a[0].emoji, emoji);
        assert_eq!(groups_a[0].count, 1);

        let groups_b = db
            .get_reactions(community_b, &event_id, event_created_at, 100, None)
            .await
            .expect("get reactions B");
        assert!(
            groups_b.is_empty(),
            "B must NOT see A's reaction for the same event shape, got {groups_b:?}"
        );

        // (3) Active-record lookup is scoped: present on A, absent on B.
        assert!(
            db.get_active_reaction_record(community_a, &event_id, event_created_at, &pubkey, emoji)
                .await
                .expect("active record A")
                .is_some(),
            "A's active reaction record must be present"
        );
        assert!(
            db.get_active_reaction_record(community_b, &event_id, event_created_at, &pubkey, emoji)
                .await
                .expect("active record B")
                .is_none(),
            "B must not find A's active reaction record"
        );

        // (4) B can add the identical shape independently (no PK collision).
        assert!(
            db.add_reaction(
                community_b,
                &event_id,
                event_created_at,
                &pubkey,
                emoji,
                None
            )
            .await
            .expect("add reaction under B"),
            "B must be able to add the same shape as its own scoped row"
        );

        // (5) Removing from B does not touch A's row.
        assert!(
            db.remove_reaction(community_b, &event_id, event_created_at, &pubkey, emoji)
                .await
                .expect("remove under B"),
            "B remove must affect B's own row"
        );
        assert!(
            db.get_active_reaction_record(community_a, &event_id, event_created_at, &pubkey, emoji)
                .await
                .expect("active record A after B remove")
                .is_some(),
            "A's reaction must survive a B-side removal"
        );

        // (6) A remove affects only A; A's read now empty.
        assert!(
            db.remove_reaction(community_a, &event_id, event_created_at, &pubkey, emoji)
                .await
                .expect("remove under A"),
            "A remove must affect A's row"
        );
        let groups_a_after = db
            .get_reactions(community_a, &event_id, event_created_at, 100, None)
            .await
            .expect("get reactions A after remove");
        assert!(
            groups_a_after.is_empty(),
            "A's reaction must be gone after A removes it"
        );
    }
}
