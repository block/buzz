//! Thread-participation tracking for mention-gated subscriptions.
//!
//! In [`SubscribeMode::Mentions`](crate::config::SubscribeMode) the harness
//! only receives events that `#p`-tag the agent. That gate is correct for
//! channel noise, but it also drops untagged replies in threads the agent is
//! actively part of — a human answering "yes, go ahead" in the agent's own
//! thread is never delivered, and the agent goes deaf mid-conversation
//! (issue #2270).
//!
//! [`ThreadFollowState`] records the NIP-10 root ids of threads the agent has
//! been dispatched into, per channel. Roots expire after a sliding TTL of
//! inactivity and each channel is capped at [`MAX_FOLLOWED_ROOTS_PER_CHANNEL`]
//! (stalest evicted first), so both the local set and the `#e` REQ filter
//! clause built from it stay bounded.
//!
//! The set feeds two consumers:
//! - [`relay`](crate::relay): a second REQ filter clause (`#h` + `#e`) so the
//!   relay delivers untagged replies for followed threads only — the mention
//!   gate stays intact for everything else.
//! - [`filter::match_event`](crate::filter::match_event): local admission for
//!   delivered events whose thread root is followed.
//!
//! All state is in-memory, matching the rest of the harness: after a restart
//! the agent follows threads it is re-mentioned in, which is the pre-existing
//! recovery semantic for every other subscription decision.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use tracing::debug;
use uuid::Uuid;

/// Upper bound on followed thread roots per channel.
///
/// Bounds both memory and the `#e` array length in the REQ filter clause
/// (relays cap filter sizes). When the cap is hit, the least-recently-active
/// root is evicted — the agent simply needs a fresh mention to rejoin that
/// thread, which is the pre-fix behavior for all threads.
pub const MAX_FOLLOWED_ROOTS_PER_CHANNEL: usize = 64;

/// Tracks which thread roots the agent participates in, per channel.
///
/// A root is recorded when a batch is dispatched to the agent (the agent is
/// about to speak in that thread) and refreshed whenever a followed-thread
/// event is admitted, giving active conversations a sliding TTL.
#[derive(Debug)]
pub struct ThreadFollowState {
    /// When `false` every method is a no-op and [`live_roots`](Self::live_roots)
    /// is always empty — call sites never need their own gating.
    enabled: bool,
    ttl: Duration,
    /// channel → (thread root event id, hex) → last-activity instant.
    followed: HashMap<Uuid, HashMap<String, Instant>>,
    /// Channels whose live root *set* changed since the last
    /// [`take_dirty`](Self::take_dirty) — i.e. the `#e` REQ clause is stale.
    dirty: HashSet<Uuid>,
    /// Last periodic compaction, throttled to [`COMPACT_INTERVAL`].
    last_compact: Instant,
}

/// Minimum interval between periodic compactions — the maintenance tick fires
/// far more often than expiry resolution requires.
const COMPACT_INTERVAL: Duration = Duration::from_secs(30);

impl ThreadFollowState {
    /// Create a tracker whose roots expire after `ttl_secs` of inactivity.
    /// A disabled tracker never records or reports anything.
    pub fn new(ttl_secs: u64, enabled: bool) -> Self {
        Self {
            enabled,
            ttl: Duration::from_secs(ttl_secs),
            followed: HashMap::new(),
            dirty: HashSet::new(),
            last_compact: Instant::now(),
        }
    }

    /// Whether thread-following is active.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Record participation in `root_id` on `channel_id`, refreshing its TTL.
    ///
    /// Marks the channel dirty when the root is new (the REQ clause must be
    /// widened). Evicts the least-recently-active root when the per-channel
    /// cap is exceeded.
    pub fn record(&mut self, channel_id: Uuid, root_id: &str) {
        self.record_at(channel_id, root_id, Instant::now());
    }

    fn record_at(&mut self, channel_id: Uuid, root_id: &str, now: Instant) {
        if !self.enabled {
            return;
        }
        let roots = self.followed.entry(channel_id).or_default();
        let is_new = !roots.contains_key(root_id);
        roots.insert(root_id.to_string(), now);

        if roots.len() > MAX_FOLLOWED_ROOTS_PER_CHANNEL {
            // Evict the stalest root. `max_by_key` on elapsed == oldest instant.
            if let Some(stalest) = roots
                .iter()
                .min_by_key(|(_, seen)| **seen)
                .map(|(root, _)| root.clone())
            {
                roots.remove(&stalest);
                debug!(
                    channel = %channel_id,
                    evicted_root = %stalest,
                    cap = MAX_FOLLOWED_ROOTS_PER_CHANNEL,
                    "followed-thread cap reached — evicted least-recently-active root"
                );
                self.dirty.insert(channel_id);
            }
        }

        if is_new {
            debug!(channel = %channel_id, root = %root_id, "following thread");
            self.dirty.insert(channel_id);
        }
    }

    /// Refresh the TTL of `root_id` if it is currently followed.
    ///
    /// Called when a followed-thread event is admitted, so active
    /// conversations keep sliding forward instead of expiring mid-exchange.
    pub fn touch(&mut self, channel_id: Uuid, root_id: &str) {
        self.touch_at(channel_id, root_id, Instant::now());
    }

    fn touch_at(&mut self, channel_id: Uuid, root_id: &str, now: Instant) {
        if let Some(seen) = self
            .followed
            .get_mut(&channel_id)
            .and_then(|roots| roots.get_mut(root_id))
        {
            *seen = now;
        }
    }

    /// Live (unexpired) root ids for `channel_id`, for local admission checks.
    pub fn live_roots(&self, channel_id: Uuid) -> HashSet<String> {
        self.live_roots_at(channel_id, Instant::now())
    }

    fn live_roots_at(&self, channel_id: Uuid, now: Instant) -> HashSet<String> {
        self.followed
            .get(&channel_id)
            .map(|roots| {
                roots
                    .iter()
                    .filter(|(_, seen)| now.duration_since(**seen) < self.ttl)
                    .map(|(root, _)| root.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Live root ids for `channel_id` in deterministic order, for building the
    /// `#e` REQ filter clause.
    pub fn roots_for_filter(&self, channel_id: Uuid) -> Vec<String> {
        let mut roots: Vec<String> = self.live_roots(channel_id).into_iter().collect();
        roots.sort_unstable();
        roots
    }

    /// Drop expired roots, throttled to [`COMPACT_INTERVAL`]. Channels whose
    /// set shrank are marked dirty so their REQ clause can be narrowed. Safe
    /// to call on every maintenance tick.
    pub fn compact_if_due(&mut self) {
        let now = Instant::now();
        if !self.enabled || now.duration_since(self.last_compact) < COMPACT_INTERVAL {
            return;
        }
        self.last_compact = now;
        self.compact_at(now);
    }

    fn compact_at(&mut self, now: Instant) {
        let ttl = self.ttl;
        let dirty = &mut self.dirty;
        self.followed.retain(|channel_id, roots| {
            let before = roots.len();
            roots.retain(|_, seen| now.duration_since(*seen) < ttl);
            if roots.len() != before {
                dirty.insert(*channel_id);
            }
            !roots.is_empty()
        });
    }

    /// Forget all state for a channel (e.g. after membership removal).
    ///
    /// Does not mark the channel dirty — the caller is tearing the
    /// subscription down entirely, so there is no REQ left to update.
    pub fn clear_channel(&mut self, channel_id: Uuid) {
        self.followed.remove(&channel_id);
        self.dirty.remove(&channel_id);
    }

    /// Drain the set of channels whose followed-root set changed since the
    /// last call. Each returned channel needs its subscription re-issued with
    /// a fresh `#e` clause.
    pub fn take_dirty(&mut self) -> Vec<Uuid> {
        self.dirty.drain().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn record_marks_new_roots_dirty_once() {
        let mut state = ThreadFollowState::new(60, true);
        state.record(ch(1), "root-a");
        assert_eq!(state.take_dirty(), vec![ch(1)]);

        // Re-recording the same root refreshes TTL but is not a set change.
        state.record(ch(1), "root-a");
        assert!(state.take_dirty().is_empty());

        assert!(state.live_roots(ch(1)).contains("root-a"));
    }

    #[test]
    fn roots_expire_after_ttl_and_mark_dirty() {
        let mut state = ThreadFollowState::new(60, true);
        let start = Instant::now();
        state.record_at(ch(1), "root-a", start);
        state.take_dirty();

        let later = start + Duration::from_secs(61);
        assert!(state.live_roots_at(ch(1), later).is_empty());

        state.compact_at(later);
        assert_eq!(state.take_dirty(), vec![ch(1)]);
        assert!(state.roots_for_filter(ch(1)).is_empty());
    }

    #[test]
    fn touch_slides_expiry_forward() {
        let mut state = ThreadFollowState::new(60, true);
        let start = Instant::now();
        state.record_at(ch(1), "root-a", start);

        // Refresh at t+45; at t+75 the root is still live (75 - 45 < 60).
        state.touch_at(ch(1), "root-a", start + Duration::from_secs(45));
        let at_75 = start + Duration::from_secs(75);
        assert!(state.live_roots_at(ch(1), at_75).contains("root-a"));

        // Without further activity it expires at t+45+60.
        let at_106 = start + Duration::from_secs(106);
        assert!(state.live_roots_at(ch(1), at_106).is_empty());
    }

    #[test]
    fn touch_ignores_unknown_roots() {
        let mut state = ThreadFollowState::new(60, true);
        state.touch(ch(1), "never-recorded");
        assert!(state.live_roots(ch(1)).is_empty());
        assert!(state.take_dirty().is_empty());
    }

    #[test]
    fn cap_evicts_least_recently_active_root() {
        let mut state = ThreadFollowState::new(3600, true);
        let start = Instant::now();
        for i in 0..MAX_FOLLOWED_ROOTS_PER_CHANNEL {
            state.record_at(
                ch(1),
                &format!("root-{i:03}"),
                start + Duration::from_secs(i as u64),
            );
        }
        state.take_dirty();

        // One over the cap evicts root-000 (the stalest), keeps the newcomer.
        state.record_at(
            ch(1),
            "root-new",
            start + Duration::from_secs(MAX_FOLLOWED_ROOTS_PER_CHANNEL as u64),
        );
        let live = state.live_roots_at(ch(1), start + Duration::from_secs(100));
        assert_eq!(live.len(), MAX_FOLLOWED_ROOTS_PER_CHANNEL);
        assert!(!live.contains("root-000"));
        assert!(live.contains("root-new"));
        assert_eq!(state.take_dirty(), vec![ch(1)]);
    }

    #[test]
    fn channels_are_independent() {
        let mut state = ThreadFollowState::new(60, true);
        state.record(ch(1), "root-a");
        state.record(ch(2), "root-b");

        assert!(state.live_roots(ch(1)).contains("root-a"));
        assert!(!state.live_roots(ch(1)).contains("root-b"));

        state.clear_channel(ch(1));
        assert!(state.live_roots(ch(1)).is_empty());
        assert!(state.live_roots(ch(2)).contains("root-b"));
        // clear_channel drops pending dirtiness for the removed channel.
        assert_eq!(state.take_dirty(), vec![ch(2)]);
    }

    #[test]
    fn roots_for_filter_is_sorted() {
        let mut state = ThreadFollowState::new(60, true);
        state.record(ch(1), "bbb");
        state.record(ch(1), "aaa");
        state.record(ch(1), "ccc");
        assert_eq!(state.roots_for_filter(ch(1)), vec!["aaa", "bbb", "ccc"]);
    }
}
