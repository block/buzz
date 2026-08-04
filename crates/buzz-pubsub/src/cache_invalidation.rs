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
//!
//! # Subscription scope
//!
//! This pod subscribes to the exact per-community channels of the communities
//! whose entries it may hold — **cache residency**, not connection lifetime.
//! Cached authorization outlives the socket that populated it, so scoping by
//! live connections would leave a pod holding stale authz for a community it is
//! no longer listening to. See [`crate::community_topics`] for the desired /
//! established contract and why the caller must not insert a cache entry for a
//! community that is not yet established.

use std::sync::Arc;

use buzz_core::{CommunityId, TenantContext};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::community_topics::{
    run_community_subscriber, CommunityChannelFamily, CommunityTopics, DesiredCommunities,
};
use crate::topic::BUZZ_PREFIX;

/// Tenant-local Redis pub/sub channel suffix for cache-invalidation messages.
pub const CACHE_INVALIDATION_SUFFIX: &str = "cache-invalidate";

/// Log label for the cache-invalidation subscriber.
pub(crate) const CACHE_INVALIDATION_NAME: &str = "cache-invalidation";

/// Redis pub/sub channel for cache-invalidation messages under `ctx`.
pub fn cache_invalidation_channel(ctx: &TenantContext) -> String {
    cache_invalidation_channel_for(ctx.community())
}

/// Redis pub/sub channel for cache-invalidation messages in `community`.
///
/// The subscriber needs this without a [`TenantContext`]: it subscribes to the
/// exact channels of the communities whose cache entries this pod holds, and
/// those are known only as ids.
pub fn cache_invalidation_channel_for(community: CommunityId) -> String {
    format!("{BUZZ_PREFIX}:{community}:{CACHE_INVALIDATION_SUFFIX}")
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

/// Subscribes to the exact `buzz:{community}:cache-invalidate` channels this pod
/// needs and forwards scoped drops to the broadcast.
///
/// `desired` is the set of communities whose cache entries this pod may hold —
/// re-read on every reconcile, never cached here. Never returns; runs for the
/// lifetime of the relay. See [`crate::community_topics`] for the level-triggered
/// reconciliation and establishment contract.
pub async fn run_cache_invalidation_subscriber(
    redis_url: String,
    broadcast_tx: broadcast::Sender<ScopedCacheInvalidation>,
    topics: Arc<CommunityTopics>,
    desired: DesiredCommunities,
) {
    run_community_subscriber(
        redis_url,
        CacheInvalidationFamily { broadcast_tx },
        topics,
        desired,
    )
    .await;
}

struct CacheInvalidationFamily {
    broadcast_tx: broadcast::Sender<ScopedCacheInvalidation>,
}

impl CommunityChannelFamily for CacheInvalidationFamily {
    fn channel(&self, community: CommunityId) -> String {
        cache_invalidation_channel_for(community)
    }

    fn parse_channel(&self, channel: &str) -> Option<CommunityId> {
        parse_cache_invalidation_channel(channel)
    }

    fn deliver(&self, community_id: CommunityId, payload: &str) {
        let invalidation: CacheInvalidation = match serde_json::from_str(payload) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("Failed to deserialize cache-invalidation message: {e}");
                return;
            }
        };

        let scoped = ScopedCacheInvalidation {
            community_id,
            invalidation,
        };

        if self.broadcast_tx.send(scoped).is_err() {
            tracing::trace!("No cache-invalidation receivers — message dropped");
        }
    }
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
