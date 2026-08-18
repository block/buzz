//! Cross-pod cache-key invalidation over Redis pub/sub.
//!
//! Each relay pod keeps in-memory (moka) membership / accessible-channels /
//! visibility caches. A membership or visibility change is applied to the local
//! caches only on the pod that processed the write; other pods would otherwise
//! rely on the 10s TTL to expire stale entries. This module carries the same
//! key drops to every pod immediately.
//!
//! The message is a pure cache-key drop — never an "evict these subscriptions"
//! payload. The per-event access gate (`filter_fanout_by_access`) is the
//! universal delivery-enforcement point, so dropping the stale key is
//! sufficient: the next read re-fetches authoritative state from the DB.

use buzz_core::{CommunityId, TenantContext};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::topic::BUZZ_PREFIX;

/// Tenant-local Redis pub/sub channel suffix for cache-invalidation messages.
pub const CACHE_INVALIDATION_SUFFIX: &str = "cache-invalidate";

/// Pattern used by the subscriber to receive cache invalidations for all
/// communities this pod may have cached locally.
pub const CACHE_INVALIDATION_PATTERN: &str = "buzz:*:cache-invalidate";

/// Redis pub/sub channel for cache-invalidation messages under `ctx`.
pub fn cache_invalidation_channel(ctx: &TenantContext) -> String {
    format!(
        "{BUZZ_PREFIX}:{}:{CACHE_INVALIDATION_SUFFIX}",
        ctx.community()
    )
}

/// Parse a cache-invalidation Redis channel into its scoped community id.
pub fn parse_cache_invalidation_channel(channel: &str) -> Option<CommunityId> {
    let mut parts = channel.split(':');
    if parts.next()? != BUZZ_PREFIX {
        return None;
    }
    let community_id = Uuid::parse_str(parts.next()?).ok()?;
    if parts.next()? != CACHE_INVALIDATION_SUFFIX {
        return None;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(CommunityId::from_uuid(community_id))
}

/// A cache-key drop to apply on every pod. Each variant mirrors exactly one of
/// the relay's local `invalidate_*` operations. The community is carried by
/// [`ScopedCacheInvalidation`], not by the tenant-local operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op")]
pub enum CacheInvalidation {
    /// Drop the `(channel_id, pubkey)` membership entry and the user's
    /// accessible-channels entry. Mirrors `invalidate_membership`.
    Membership {
        /// Channel whose membership changed.
        channel_id: Uuid,
        /// Affected member's pubkey bytes.
        pubkey: Vec<u8>,
    },
    /// Drop every user's accessible-channels entry. Mirrors
    /// `invalidate_all_accessible_channels` (e.g. a new open channel).
    AccessibleAll,
    /// Drop the cached visibility for a single channel. Mirrors
    /// `invalidate_channel_visibility` (e.g. an open→private flip).
    Visibility {
        /// Channel whose visibility changed.
        channel_id: Uuid,
    },
    /// Drop all membership / accessible / visibility caches. Mirrors
    /// `invalidate_channel_deleted`.
    ChannelDeleted,
}

/// A cache invalidation received from a community-scoped Redis channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedCacheInvalidation {
    /// Community whose local cache key should be dropped.
    pub community_id: CommunityId,
    /// Tenant-local cache invalidation operation.
    pub invalidation: CacheInvalidation,
}

/// Initial reconnect backoff (1 second).
const BACKOFF_INITIAL_SECS: u64 = 1;
/// Maximum reconnect backoff (30 seconds).
const BACKOFF_MAX_SECS: u64 = 30;

/// Subscribes to `buzz:*:cache-invalidate` and forwards scoped drops to the broadcast.
///
/// Mirrors `subscriber::run_subscriber`: a reconnect loop with exponential
/// backoff (1s → 2s → 4s → … → 30s max). Never returns — runs for the lifetime
/// of the relay.
pub async fn run_cache_invalidation_subscriber(
    redis_url: String,
    broadcast_tx: broadcast::Sender<ScopedCacheInvalidation>,
) {
    let mut backoff_secs = BACKOFF_INITIAL_SECS;

    loop {
        match connect_and_subscribe(&redis_url, &broadcast_tx).await {
            Ok(()) => {
                backoff_secs = BACKOFF_INITIAL_SECS;
                tracing::warn!(
                    "Redis cache-invalidation stream ended (clean disconnect) — reconnecting in {backoff_secs}s"
                );
            }
            Err(e) => {
                tracing::error!(
                    "Redis cache-invalidation error: {e} — reconnecting in {backoff_secs}s"
                );
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(backoff_secs)).await;
        backoff_secs = (backoff_secs * 2).min(BACKOFF_MAX_SECS);

        tracing::info!("Attempting to reconnect to Redis cache-invalidation...");
    }
}

async fn connect_and_subscribe(
    redis_url: &str,
    broadcast_tx: &broadcast::Sender<ScopedCacheInvalidation>,
) -> Result<(), redis::RedisError> {
    let client = redis::Client::open(redis_url)?;
    let mut conn = client.get_async_pubsub().await?;

    conn.psubscribe(CACHE_INVALIDATION_PATTERN).await?;

    tracing::info!(
        "Redis cache-invalidation subscriber connected — listening on {CACHE_INVALIDATION_PATTERN}"
    );

    let mut stream = conn.on_message();
    while let Some(msg) = stream.next().await {
        let channel = msg.get_channel_name();
        let Some(community_id) = parse_cache_invalidation_channel(channel) else {
            tracing::warn!("Received cache-invalidation message on unexpected channel: {channel}");
            continue;
        };

        let payload: String = match msg.get_payload() {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Failed to get cache-invalidation payload: {e}");
                continue;
            }
        };

        let invalidation: CacheInvalidation = match serde_json::from_str(&payload) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("Failed to deserialize cache-invalidation message: {e}");
                continue;
            }
        };

        let scoped = ScopedCacheInvalidation {
            community_id,
            invalidation,
        };

        if broadcast_tx.send(scoped).is_err() {
            tracing::trace!("No cache-invalidation receivers — message dropped");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(id: u128, host: &str) -> TenantContext {
        TenantContext::resolved(CommunityId::from_uuid(Uuid::from_u128(id)), host)
    }

    #[test]
    fn cache_invalidation_channel_is_community_scoped() {
        let community_a = ctx(0xaaaa, "a.example");
        let community_b = ctx(0xbbbb, "b.example");

        assert_eq!(
            cache_invalidation_channel(&community_a),
            format!("buzz:{}:cache-invalidate", community_a.community())
        );
        assert_ne!(
            cache_invalidation_channel(&community_a),
            cache_invalidation_channel(&community_b)
        );
    }

    #[test]
    fn parses_cache_invalidation_channel() {
        let community_id = CommunityId::from_uuid(Uuid::from_u128(0xaaaa));
        let raw = format!("buzz:{community_id}:cache-invalidate");

        assert_eq!(parse_cache_invalidation_channel(&raw), Some(community_id));
    }

    #[test]
    fn rejects_bad_cache_invalidation_channels() {
        for raw in [
            "buzz:cache-invalidate",
            "buzz:not-a-uuid:cache-invalidate",
            "not-buzz:00000000-0000-0000-0000-00000000aaaa:cache-invalidate",
            "buzz:00000000-0000-0000-0000-00000000aaaa:cache-invalidate:extra",
            "buzz:00000000-0000-0000-0000-00000000aaaa:channel:00000000-0000-0000-0000-00000000bbbb",
        ] {
            assert_eq!(parse_cache_invalidation_channel(raw), None);
        }
    }

    #[test]
    fn membership_roundtrips_through_json() {
        let msg = CacheInvalidation::Membership {
            channel_id: Uuid::from_u128(0x1234),
            pubkey: vec![1, 2, 3, 4],
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(
            serde_json::from_str::<CacheInvalidation>(&json).unwrap(),
            msg
        );
    }

    #[test]
    fn unit_variants_roundtrip_through_json() {
        for msg in [
            CacheInvalidation::AccessibleAll,
            CacheInvalidation::ChannelDeleted,
            CacheInvalidation::Visibility {
                channel_id: Uuid::from_u128(0xabcd),
            },
        ] {
            let json = serde_json::to_string(&msg).unwrap();
            assert_eq!(
                serde_json::from_str::<CacheInvalidation>(&json).unwrap(),
                msg
            );
        }
    }
}
