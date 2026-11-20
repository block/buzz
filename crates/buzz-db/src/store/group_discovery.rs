//! Finding channels whose relay-signed discovery events are missing.
//!
//! The relay publishes a channel's kind:39000 (metadata), kind:39001
//! (admins) and kind:39002 (members) after the channel's own write commits,
//! so a failure in between leaves a healthy channel that clients cannot
//! confirm. These reads find such channels so the relay can publish them
//! again.

use buzz_core::CommunityId;
use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::{observability, Db, Result};

/// The relay-signed kinds every live channel has one live event of.
pub const GROUP_DISCOVERY_KINDS: [i32; 3] = [39000, 39001, 39002];

/// A live channel with at least one of [`GROUP_DISCOVERY_KINDS`] missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelMissingDiscovery {
    /// Community that owns the channel.
    pub community_id: CommunityId,
    /// Canonical host for the owning community.
    pub host: String,
    /// The channel.
    pub channel_id: Uuid,
    /// When the channel was created; with `channel_id`, the scan's cursor.
    pub created_at: DateTime<Utc>,
}

// One probe per kind on `idx_events_addressable`. A channel's discovery
// events are written after its row, so `created_at` bounds them from below
// and, at run time, prunes the monthly `events` partitions to the ones since
// the channel was created. The hour of slack covers a relay clock behind the
// database's: events carry the relay's time, the channel row the database's.
// `$also` adds further conditions on `e`, such as a constant floor the
// planner can prune with whatever join it picks.
//
// Expands to the probe as a string literal, with the kinds array and relay
// pubkey as the given bind parameters, so each query stays static SQL.
macro_rules! missing_kind_probe {
    ($kinds:literal, $relay:literal, $also:literal) => {
        concat!(
            "SELECT 1 FROM unnest(",
            $kinds,
            "::int[]) AS wanted(kind)
             WHERE NOT EXISTS (
                 SELECT 1 FROM events e
                 WHERE e.community_id = ch.community_id
                   AND e.kind = wanted.kind
                   AND e.pubkey = ",
            $relay,
            "
                   AND e.channel_id = ch.id
                   AND e.deleted_at IS NULL
                   AND e.created_at >= ch.created_at - INTERVAL '1 hour'
                   ",
            $also,
            "
             )"
        )
    };
}

const CHANNEL_MISSING_DISCOVERY_SQL: &str = concat!(
    "SELECT EXISTS (
        SELECT 1 FROM channels ch
        WHERE ch.community_id = $1 AND ch.id = $2 AND ch.deleted_at IS NULL
          AND EXISTS (",
    missing_kind_probe!("$4", "$3", ""),
    "))"
);

const LIST_CHANNELS_MISSING_DISCOVERY_SQL: &str = concat!(
    "SELECT ch.community_id, community.host, ch.id, ch.created_at
     FROM channels ch
     JOIN communities community
       ON community.id = ch.community_id
      AND community.archived_at IS NULL
      AND community.deleted_at IS NULL
      AND community.deletion_state = 'active'
     WHERE ch.deleted_at IS NULL
       AND ch.created_at >= $1
       AND ch.created_at < $2
       AND (ch.created_at, ch.id) > ($3, $4)
       AND EXISTS (",
    missing_kind_probe!(
        "$6",
        "$5",
        "AND e.created_at >= $1::timestamptz - INTERVAL '1 hour'"
    ),
    ")
     ORDER BY ch.created_at, ch.id
     LIMIT $7"
);

/// Whether the live channel `channel_id` lacks a live relay-signed event of
/// any of [`GROUP_DISCOVERY_KINDS`]. A deleted or unknown channel lacks
/// nothing.
pub async fn channel_missing_discovery(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    relay_pubkey: &[u8],
) -> Result<bool> {
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::EventWrite).await?;
    let missing: bool = sqlx::query_scalar(CHANNEL_MISSING_DISCOVERY_SQL)
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .bind(relay_pubkey)
        .bind(GROUP_DISCOVERY_KINDS.as_slice())
        .fetch_one(&mut *connection)
        .await?;
    Ok(missing)
}

/// One page, in `(created_at, id)` order, of live channels in active
/// communities created from `created_since` until `created_before` that
/// lack a live relay-signed event of any of [`GROUP_DISCOVERY_KINDS`].
///
/// `after` is the previous page's last `(created_at, channel_id)`. A page
/// shorter than `limit` is the last one. This is an internal
/// cross-community maintenance read: callers must keep each channel's
/// community when publishing for it.
pub async fn list_channels_missing_discovery(
    pool: &PgPool,
    relay_pubkey: &[u8],
    created_since: DateTime<Utc>,
    created_before: DateTime<Utc>,
    after: Option<(DateTime<Utc>, Uuid)>,
    limit: i64,
) -> Result<Vec<ChannelMissingDiscovery>> {
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Maintenance).await?;
    let (after_created_at, after_id) = after.unwrap_or((created_since, Uuid::nil()));
    let rows = sqlx::query(LIST_CHANNELS_MISSING_DISCOVERY_SQL)
        .bind(created_since)
        .bind(created_before)
        .bind(after_created_at)
        .bind(after_id)
        .bind(relay_pubkey)
        .bind(GROUP_DISCOVERY_KINDS.as_slice())
        .bind(limit)
        .fetch_all(&mut *connection)
        .await?;
    rows.into_iter()
        .map(|row| {
            Ok(ChannelMissingDiscovery {
                community_id: CommunityId::from_uuid(row.try_get("community_id")?),
                host: row.try_get("host")?,
                channel_id: row.try_get("id")?,
                created_at: row.try_get("created_at")?,
            })
        })
        .collect()
}

impl Db {
    /// Whether a live channel lacks any of its relay-signed discovery events.
    /// See [`channel_missing_discovery`].
    #[datastore_span(name = "channel_missing_discovery", system = "postgresql")]
    pub async fn channel_missing_discovery(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        relay_pubkey: &[u8],
    ) -> Result<bool> {
        channel_missing_discovery(&self.pool, community_id, channel_id, relay_pubkey).await
    }

    /// One page of channels lacking relay-signed discovery events. See
    /// [`list_channels_missing_discovery`].
    #[datastore_span(name = "list_channels_missing_discovery", system = "postgresql")]
    pub async fn list_channels_missing_discovery(
        &self,
        relay_pubkey: &[u8],
        created_since: DateTime<Utc>,
        created_before: DateTime<Utc>,
        after: Option<(DateTime<Utc>, Uuid)>,
        limit: i64,
    ) -> Result<Vec<ChannelMissingDiscovery>> {
        list_channels_missing_discovery(
            &self.pool,
            relay_pubkey,
            created_since,
            created_before,
            after,
            limit,
        )
        .await
    }
}
