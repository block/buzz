//! One active-membership snapshot per community and user, shared by all of
//! their channel checks. Discovery lists have different channel-type rules.

use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use buzz_core::CommunityId;
use buzz_db::{DbError, ReadAt};
use tokio::sync::Mutex;
use uuid::Uuid;

type Key = (CommunityId, Vec<u8>);
type Snapshot = ReadAt<HashSet<Uuid>>;
type Entry = Arc<Mutex<Option<Snapshot>>>;

const MAX_AGE: Duration = Duration::from_secs(10);

/// Invalidation removes the entire entry, including its read gate. A reader
/// already holding that entry can finish, but cannot restore it to the map.
/// Thus a read racing a removal never seeds later requests with its old set.
pub(crate) struct MembershipCache {
    entries: moka::sync::Cache<Key, Entry>,
    wait: Duration,
}

impl MembershipCache {
    pub(crate) fn new(wait: Duration) -> Self {
        Self {
            entries: moka::sync::Cache::builder()
                .max_capacity(10_000)
                .time_to_idle(Duration::from_secs(60))
                .support_invalidation_closures()
                .build(),
            wait,
        }
    }

    /// Shares one successful load across the user's channel checks. Errors
    /// are not cached; cancellation releases the gate. Waiters are bounded by
    /// the pool's acquire timeout plus the shared-read statement allowance.
    pub(crate) async fn contains(
        &self,
        key: Key,
        channel: Uuid,
        read: impl Future<Output = Result<ReadAt<Vec<Uuid>>, DbError>>,
    ) -> Result<bool, DbError> {
        let entry = self.entries.get_with(key, || Arc::new(Mutex::new(None)));
        let mut guard = tokio::time::timeout(self.wait, entry.lock()).await.ok();
        if let Some(snapshot) = guard.as_deref().and_then(|value| value.as_ref()) {
            if snapshot.read_at.elapsed() < MAX_AGE {
                metrics::counter!("buzz_membership_cache_hits_total").increment(1);
                return Ok(snapshot.value.contains(&channel));
            }
        }

        metrics::counter!("buzz_membership_cache_misses_total").increment(1);
        let read = read.await?;
        let snapshot = ReadAt {
            value: read.value.into_iter().collect::<HashSet<_>>(),
            read_at: read.read_at,
        };
        let member = snapshot.value.contains(&channel);
        if let Some(guard) = guard.as_deref_mut() {
            *guard = Some(snapshot);
        }
        // A timed-out waiter does an uncached read. It cannot overwrite the
        // gate holder's newer snapshot if its own query completes later.
        Ok(member)
    }

    pub(crate) fn invalidate(&self, community: CommunityId, pubkey: &[u8]) {
        self.entries.invalidate(&(community, pubkey.to_vec()));
    }

    pub(crate) fn invalidate_community(&self, community: CommunityId) {
        if let Err(error) = self
            .entries
            .invalidate_entries_if(move |(entry_community, _), _| *entry_community == community)
        {
            tracing::error!(
                ?error,
                "membership invalidation unavailable; dropping all entries"
            );
            self.entries.invalidate_all();
        }
    }

    /// Seed one membership in a test's synthetic snapshot.
    #[cfg(test)]
    pub(crate) fn seed_member(&self, key: (CommunityId, Uuid, Vec<u8>), member: bool) {
        let (community, channel, pubkey) = key;
        let entry = self
            .entries
            .get_with((community, pubkey), || Arc::new(Mutex::new(None)));
        let mut guard = entry.try_lock().expect("test entry must not be loading");
        let snapshot = guard.get_or_insert_with(|| ReadAt {
            value: HashSet::new(),
            read_at: std::time::Instant::now(),
        });
        if member {
            snapshot.value.insert(channel);
        } else {
            snapshot.value.remove(&channel);
        }
    }

    #[cfg(test)]
    pub(crate) fn cached_member(&self, key: &(CommunityId, Uuid, Vec<u8>)) -> Option<bool> {
        let entry = self.entries.get(&(key.0, key.2.clone()))?;
        let guard = entry.try_lock().ok()?;
        guard
            .as_ref()
            .filter(|snapshot| snapshot.read_at.elapsed() < MAX_AGE)
            .map(|snapshot| snapshot.value.contains(&key.1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use tokio::sync::oneshot;

    fn community(n: u128) -> CommunityId {
        CommunityId::from_uuid(Uuid::from_u128(n))
    }

    fn snapshot(ids: Vec<Uuid>) -> ReadAt<Vec<Uuid>> {
        ReadAt {
            value: ids,
            read_at: Instant::now(),
        }
    }

    #[tokio::test]
    async fn concurrent_channels_share_one_membership_read() {
        let cache = Arc::new(MembershipCache::new(Duration::from_secs(5)));
        let key = (community(1), vec![1; 32]);
        let channels = (1..=280).map(Uuid::from_u128).collect::<Vec<_>>();
        let (entered, started) = oneshot::channel();
        let (release, released) = oneshot::channel();
        let leader = {
            let cache = cache.clone();
            let key = key.clone();
            let channels = channels.clone();
            tokio::spawn(async move {
                cache
                    .contains(key, channels[0], async {
                        entered.send(()).unwrap();
                        released.await.unwrap();
                        Ok(snapshot(channels))
                    })
                    .await
            })
        };
        started.await.unwrap();
        let mut waiters = channels
            .into_iter()
            .map(|channel| {
                Box::pin(cache.contains(key.clone(), channel, async {
                    panic!("duplicate database read")
                }))
            })
            .collect::<Vec<_>>();
        // Poll each future into the held gate before releasing the leader.
        // This does not depend on scheduling or moka's internal Arc clones.
        for waiter in &mut waiters {
            assert!(futures_util::poll!(waiter.as_mut()).is_pending());
        }
        release.send(()).unwrap();
        assert!(leader.await.unwrap().unwrap());
        for waiter in waiters {
            assert!(waiter.await.unwrap());
        }
        assert!(!cache
            .contains(key, Uuid::from_u128(281), async {
                panic!("negative membership also comes from the snapshot")
            })
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn invalidation_detaches_an_in_flight_snapshot() {
        for whole_community in [false, true] {
            let cache = Arc::new(MembershipCache::new(Duration::from_secs(5)));
            let key = (community(1), vec![1; 32]);
            let channel = Uuid::from_u128(1);
            let other = (community(2), channel, key.1.clone());
            cache.seed_member(other.clone(), true);
            let (entered, started) = oneshot::channel();
            let (release, released) = oneshot::channel();
            let old_read = {
                let cache = cache.clone();
                let key = key.clone();
                tokio::spawn(async move {
                    cache
                        .contains(key, channel, async {
                            let old = snapshot(vec![channel]);
                            entered.send(()).unwrap();
                            released.await.unwrap();
                            Ok(old)
                        })
                        .await
                })
            };
            started.await.unwrap();
            if whole_community {
                cache.invalidate_community(key.0);
            } else {
                cache.invalidate(key.0, &key.1);
            }
            assert!(!cache
                .contains(key.clone(), channel, async { Ok(snapshot(vec![])) })
                .await
                .unwrap());
            release.send(()).unwrap();
            assert!(
                old_read.await.unwrap().unwrap(),
                "the original in-flight read may finish"
            );
            assert!(!cache
                .contains(key, channel, async { panic!("new entry was overwritten") })
                .await
                .unwrap());
            assert_eq!(cache.cached_member(&other), Some(true));
        }
    }

    #[tokio::test]
    async fn stale_reads_and_database_errors_are_not_reused() {
        let cache = MembershipCache::new(Duration::from_secs(1));
        let key = (community(1), vec![1; 32]);
        let channel = Uuid::from_u128(1);
        assert!(cache
            .contains(key.clone(), channel, async {
                Err(DbError::InvalidData("unavailable".into()))
            })
            .await
            .is_err());
        assert!(cache
            .contains(key.clone(), channel, async {
                Ok(ReadAt {
                    value: vec![channel],
                    read_at: Instant::now() - MAX_AGE,
                })
            })
            .await
            .unwrap());
        assert!(!cache
            .contains(key.clone(), channel, async { Ok(snapshot(vec![])) })
            .await
            .unwrap());
        cache.invalidate(key.0, &key.1);
        assert!(
            cache
                .contains(key, channel, async { Ok(snapshot(vec![channel])) })
                .await
                .unwrap(),
            "a grant invalidates a cached denial"
        );
    }

    #[tokio::test]
    async fn cancelling_a_loader_releases_its_waiters() {
        let cache = Arc::new(MembershipCache::new(Duration::from_secs(5)));
        let key = (community(1), vec![1; 32]);
        let channel = Uuid::from_u128(1);
        let (entered, started) = oneshot::channel();
        let leader = {
            let cache = cache.clone();
            let key = key.clone();
            tokio::spawn(async move {
                cache
                    .contains(key, channel, async {
                        entered.send(()).unwrap();
                        std::future::pending().await
                    })
                    .await
            })
        };
        started.await.unwrap();
        let waiter = cache.contains(key, channel, async { Ok(snapshot(vec![channel])) });
        tokio::pin!(waiter);
        assert!(futures_util::poll!(&mut waiter).is_pending());
        leader.abort();
        assert!(leader.await.unwrap_err().is_cancelled());
        assert!(tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .unwrap()
            .unwrap());
    }

    #[tokio::test(start_paused = true)]
    async fn a_hung_loader_does_not_hold_waiters_or_let_them_overwrite_it() {
        let wait = Duration::from_secs(2);
        let cache = MembershipCache::new(wait);
        let key = (community(1), vec![1; 32]);
        let channel = Uuid::from_u128(1);
        let entry = cache
            .entries
            .get_with(key.clone(), || Arc::new(Mutex::new(None)));
        let mut leader = entry.lock().await;
        let start = tokio::time::Instant::now();
        assert!(cache
            .contains(key.clone(), channel, async { Ok(snapshot(vec![channel])) })
            .await
            .unwrap());
        assert_eq!(start.elapsed(), wait);
        assert!(
            leader.is_none(),
            "timed-out waiter does not publish its result"
        );
        *leader = Some(ReadAt {
            value: HashSet::new(),
            read_at: Instant::now(),
        });
        drop(leader);
        assert!(!cache
            .contains(key, channel, async {
                panic!("leader's result must be retained")
            })
            .await
            .unwrap());
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use buzz_pubsub::cache_invalidation::CacheInvalidation;

    #[tokio::test(flavor = "current_thread")]
    #[ignore = "requires Postgres"]
    async fn channel_checks_share_one_database_read_and_follow_membership_changes() {
        let recorder = metrics_util::debugging::DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let _recorder = metrics::set_default_local_recorder(&recorder);
        let misses = || {
            snapshotter
                .snapshot()
                .into_vec()
                .into_iter()
                .filter(|(key, ..)| key.key().name() == "buzz_membership_cache_misses_total")
                .map(|(.., value)| match value {
                    metrics_util::debugging::DebugValue::Counter(n) => n,
                    _ => 0,
                })
                .sum::<u64>()
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&crate::test_support::database_url())
            .await
            .unwrap();
        let state = crate::state::tests::test_state_with_database_pool(pool.clone()).await;
        let community = state
            .db
            .ensure_configured_community(&format!("membership-{}.local", Uuid::new_v4()))
            .await
            .unwrap()
            .id;
        let user = vec![1u8; 32];
        let channels = (0..280).map(|_| Uuid::new_v4()).collect::<Vec<_>>();
        sqlx::query("INSERT INTO channels (community_id, id, name, visibility, created_by) SELECT $1, id, id::text, 'private', $3 FROM unnest($2::uuid[]) AS id")
            .bind(community.as_uuid()).bind(&channels).bind(&user).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO channel_members (community_id, channel_id, pubkey) SELECT $1, id, $3 FROM unnest($2::uuid[]) AS id")
            .bind(community.as_uuid()).bind(&channels).bind(&user).execute(&pool).await.unwrap();

        // Hold the only connection until every channel check reaches the cache.
        let held = pool.acquire().await.unwrap();
        let mut reads = channels
            .iter()
            .map(|&channel| Box::pin(state.is_member_cached(community, channel, &user)))
            .collect::<Vec<_>>();
        for read in &mut reads {
            assert!(futures_util::poll!(read.as_mut()).is_pending());
        }
        assert_eq!(misses(), 1, "only one query may wait for the pool");
        drop(held);
        for read in reads {
            assert!(read.await.unwrap());
        }
        assert_eq!(misses(), 1, "280 channel checks share one database query");

        sqlx::query("UPDATE channel_members SET removed_at = NOW() WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3")
            .bind(community.as_uuid()).bind(channels[0]).bind(&user).execute(&pool).await.unwrap();
        // Exercise the same notification handler used on another relay pod.
        state.apply_cache_invalidation(
            community,
            CacheInvalidation::Membership {
                channel_id: channels[0],
                pubkey: user.clone(),
            },
        );
        assert!(!state
            .is_member_cached(community, channels[0], &user)
            .await
            .unwrap());
        assert!(state
            .is_member_cached(community, channels[1], &user)
            .await
            .unwrap());
        assert_eq!(misses(), 2);

        sqlx::query("UPDATE channel_members SET removed_at = NULL WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3")
            .bind(community.as_uuid()).bind(channels[0]).bind(&user).execute(&pool).await.unwrap();
        state.invalidate_membership_local(community, channels[0], &user);
        assert!(state
            .is_member_cached(community, channels[0], &user)
            .await
            .unwrap());
        assert_eq!(misses(), 3);

        sqlx::query("UPDATE channels SET deleted_at = NOW() WHERE community_id = $1 AND id = $2")
            .bind(community.as_uuid())
            .bind(channels[1])
            .execute(&pool)
            .await
            .unwrap();
        state.apply_cache_invalidation(community, CacheInvalidation::ChannelDeleted);
        assert!(!state
            .is_member_cached(community, channels[1], &user)
            .await
            .unwrap());
        assert!(state
            .is_member_cached(community, channels[0], &user)
            .await
            .unwrap());
        assert_eq!(misses(), 4);

        // Same channel ID and pubkey in another community, plus a second
        // user in this one: neither may inherit the first user's snapshot.
        let other_user = vec![2u8; 32];
        let other_community = state
            .db
            .ensure_configured_community(&format!("membership-other-{}.local", Uuid::new_v4()))
            .await
            .unwrap()
            .id;
        sqlx::query("INSERT INTO channels (community_id, id, name, visibility, created_by) VALUES ($1, $2, 'other', 'private', $3)")
            .bind(other_community.as_uuid()).bind(channels[0]).bind(&user).execute(&pool).await.unwrap();
        assert!(!state
            .is_member_cached(community, channels[0], &other_user)
            .await
            .unwrap());
        assert!(!state
            .is_member_cached(other_community, channels[0], &user)
            .await
            .unwrap());
        assert!(state
            .is_member_cached(community, channels[0], &user)
            .await
            .unwrap());

        pool.close().await;
        assert!(
            state
                .is_member_cached(community, channels[0], &user)
                .await
                .unwrap(),
            "a fresh snapshot needs no database"
        );
        state.invalidate_membership_local(community, channels[0], &user);
        assert!(
            state
                .is_member_cached(community, channels[0], &user)
                .await
                .is_err(),
            "database failure must not grant access"
        );
        assert_eq!(
            state
                .membership_cache
                .cached_member(&(community, channels[0], user)),
            None,
            "database failure must not become a cached denial either"
        );
    }
}
