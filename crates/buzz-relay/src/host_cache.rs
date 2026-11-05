//! One host snapshot for routing and FI policy, with bounded read leases.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use buzz_core::CommunityId;
use buzz_db::community_auth::{CommunityAuthPolicy, POLICY_LEASE_MAX_AGE};

#[derive(Clone, Copy)]
struct Entry {
    community: CommunityId,
    read_at: Instant,
    // A routing refresh must never renew the policy lease.
    policy: Option<(CommunityAuthPolicy, Instant)>,
}

/// Bounded host cache. Routing-only entries never supply FI policy.
pub struct HostCache {
    entries: moka::sync::Cache<String, Entry>,
    // Serialize invalidation with fill publication. A single epoch avoids an
    // unbounded map of per-host tombstones; unrelated invalidation only skips
    // publication of an in-flight fill, never changes an admission decision.
    epoch: Mutex<u64>,
}

impl HostCache {
    pub(crate) fn new() -> Self {
        Self {
            entries: moka::sync::Cache::builder()
                .max_capacity(10_000)
                .time_to_live(Duration::from_secs(10))
                .support_invalidation_closures()
                .build(),
            epoch: Mutex::new(0),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, u64> {
        self.epoch
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A routing snapshot and its original read time, for legacy routing.
    pub fn get(&self, host: &str) -> Option<(CommunityId, Instant)> {
        self.entries
            .get(host)
            .map(|entry| (entry.community, entry.read_at))
    }

    /// Populate legacy routing without making a claim about FI policy.
    pub fn insert(&self, host: String, (community, read_at): (CommunityId, Instant)) {
        let _epoch = self.lock();
        let existing = self.entries.get(&host);
        let policy = existing
            .filter(|entry| entry.community == community)
            .and_then(|entry| entry.policy)
            .filter(|(_, at)| at.elapsed() < POLICY_LEASE_MAX_AGE);
        self.entries.insert(
            host,
            Entry {
                community,
                read_at,
                policy,
            },
        );
    }

    pub(crate) fn policy(&self, host: &str) -> Option<CommunityAuthPolicy> {
        let _epoch = self.lock();
        self.entries
            .get(host)
            .and_then(|entry| entry.policy)
            .filter(|(_, at)| at.elapsed() < POLICY_LEASE_MAX_AGE)
            .map(|(policy, _)| policy)
    }

    pub(crate) fn epoch(&self) -> u64 {
        *self.lock()
    }

    #[cfg(test)]
    pub(crate) fn policy_read_started_at(&self, host: &str) -> Option<Instant> {
        self.entries
            .get(host)
            .and_then(|entry| entry.policy)
            .map(|(_, read_at)| read_at)
    }

    pub(crate) fn publish(
        &self,
        host: String,
        policy: CommunityAuthPolicy,
        read_at: Instant,
        observed_epoch: u64,
    ) -> Result<CommunityAuthPolicy, ()> {
        let epoch = self.lock();
        if *epoch != observed_epoch || read_at.elapsed() >= POLICY_LEASE_MAX_AGE {
            return Err(());
        }
        if let Some(existing) = self
            .entries
            .get(&host)
            .and_then(|entry| entry.policy)
            .filter(|(_, at)| at.elapsed() < POLICY_LEASE_MAX_AGE)
            .map(|(policy, _)| policy)
        {
            if existing.community_id == policy.community_id
                && (existing.policy_generation > policy.policy_generation
                    || (existing.requires_nip_fi && !policy.requires_nip_fi))
            {
                return Ok(existing);
            }
        }
        self.entries.insert(
            host,
            Entry {
                community: CommunityId::from_uuid(policy.community_id),
                read_at,
                policy: Some((policy, read_at)),
            },
        );
        Ok(policy)
    }

    /// Drop a host snapshot and fence every fill begun before this drop.
    pub fn invalidate(&self, host: &str) {
        let mut epoch = self.lock();
        *epoch = epoch.wrapping_add(1);
        self.entries.invalidate(host);
    }

    /// Drop all snapshots and fence in-flight fills.
    pub fn invalidate_all(&self) {
        let mut epoch = self.lock();
        *epoch = epoch.wrapping_add(1);
        self.entries.invalidate_all();
    }

    pub(crate) fn invalidate_community(&self, community: CommunityId) {
        let mut epoch = self.lock();
        *epoch = epoch.wrapping_add(1);
        if let Err(error) = self
            .entries
            .invalidate_entries_if(move |_, entry| entry.community == community)
        {
            tracing::error!(
                ?error,
                "host cache invalidation unavailable; dropping all entries"
            );
            self.entries.invalidate_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_expire_and_invalidation_fences_late_or_older_fills() {
        let cache = HostCache::new();
        let host = "team.example";
        let mut policy = CommunityAuthPolicy {
            community_id: uuid::Uuid::new_v4(),
            requires_nip_fi: false,
            policy_generation: 0,
        };
        let epoch = cache.epoch();
        assert_eq!(
            cache.publish(host.into(), policy, Instant::now(), epoch),
            Ok(policy)
        );
        assert_eq!(cache.policy(host), Some(policy));
        let lease_at = cache.entries.get(host).unwrap().policy.unwrap().1;
        cache.insert(
            host.into(),
            (CommunityId::from_uuid(policy.community_id), Instant::now()),
        );
        assert_eq!(
            cache.policy(host),
            Some(policy),
            "routing erased the policy lease"
        );
        assert_eq!(
            cache.entries.get(host).unwrap().policy.unwrap().1,
            lease_at,
            "routing renewed the policy lease"
        );
        let before_drop = cache.epoch();
        cache.invalidate(host);
        assert!(cache.policy(host).is_none());
        assert!(cache
            .publish(host.into(), policy, Instant::now(), before_drop)
            .is_err());
        policy.requires_nip_fi = true;
        policy.policy_generation = 1;
        let epoch = cache.epoch();
        assert_eq!(
            cache.publish(host.into(), policy, Instant::now(), epoch),
            Ok(policy)
        );
        let mut old = policy;
        old.requires_nip_fi = false;
        old.policy_generation = 0;
        assert_eq!(
            cache.publish(host.into(), old, Instant::now(), epoch),
            Ok(policy)
        );
        assert_eq!(cache.policy(host), Some(policy));
        // The cache age is measured from the original read, even when a slow
        // query or scheduler delivers its result after the full lease elapsed.
        let expired = Instant::now().checked_sub(Duration::from_secs(1)).unwrap();
        assert!(cache
            .publish("slow.example".into(), old, expired, epoch)
            .is_err());
        assert!(cache.policy("slow.example").is_none());
        cache.invalidate_all();
        assert!(cache
            .publish(host.into(), old, Instant::now(), epoch)
            .is_err());
        assert!(cache.policy(host).is_none());
        assert_eq!(cache.entries.policy().max_capacity(), Some(10_000));
    }
}
