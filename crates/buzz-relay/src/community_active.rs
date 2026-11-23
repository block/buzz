//! Per-community lifecycle answers ("is this community active?") shared by
//! socket admission and the ephemeral write fence.
//!
//! Every new socket asks this before it can authenticate, so in a reconnect
//! storm one community gets thousands of identical reads at once. An answer
//! is served for [`COMMUNITY_ACTIVE_MAX_AGE`] after the read that produced it
//! checked out its connection, and concurrent misses share one read.
//!
//! A cached "active" must not undo the archive sweep. Socket admission
//! registers the socket before it asks, so a sweep closes every socket that
//! registered before it, and a socket that registers after it must see the
//! community inactive. [`FencedCache::invalidate`] runs before the sweep: it
//! drops the entry and fences every fill whose read began before it, so a
//! later socket reads afresh instead of taking a pre-archive answer from the
//! cache or from a shared in-flight read.

use std::time::Duration;

use buzz_core::CommunityId;

use crate::fenced_cache::FencedCache;

/// How long a pod serves a cached community lifecycle answer after the
/// database read that produced it began, at its connection checkout (see
/// [`buzz_db::ReadAt`]). Archive and deletion on this pod drop the entry
/// before closing sockets; a pod that misses that notification admits sockets
/// for at most this long, and lifecycle revalidation closes them.
pub(crate) const COMMUNITY_ACTIVE_MAX_AGE: Duration = Duration::from_secs(1);

/// Cached `is_community_active` answers, fenced against invalidation.
pub(crate) type CommunityActiveCache = FencedCache<CommunityId, bool>;

/// A cache whose concurrent misses wait at most `shared_read_wait` for a
/// shared read (see [`crate::shared_reads::SharedReads::new`]).
pub(crate) fn new_cache(shared_read_wait: Duration) -> CommunityActiveCache {
    FencedCache::new(
        10_000,
        COMMUNITY_ACTIVE_MAX_AGE,
        shared_read_wait,
        (
            "buzz_community_active_cache_hits_total",
            "buzz_community_active_cache_misses_total",
        ),
    )
}
