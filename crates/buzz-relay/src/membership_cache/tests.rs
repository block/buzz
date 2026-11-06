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
async fn membership_check_future_stays_small_in_nested_handlers() {
    let state = crate::state::tests::test_state().await;
    let user = vec![1; 32];
    let channel = Uuid::from_u128(1);
    state
        .membership_cache
        .seed_member((community(1), channel, user.clone()), true);
    let check = state.is_member_cached(community(1), channel, &user);
    let bytes = std::mem::size_of_val(&check);
    assert!(
        bytes < 4096,
        "membership checks compose inside large handler futures: {bytes} bytes"
    );
    assert!(check.await.unwrap());
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
                .contains(
                    key,
                    channels[0],
                    async {
                        entered.send(()).unwrap();
                        released.await.unwrap();
                        Ok(snapshot(channels))
                    },
                    async { panic!("unexpected point check") },
                )
                .await
        })
    };
    started.await.unwrap();
    let mut waiters = channels
        .into_iter()
        .map(|channel| {
            Box::pin(cache.contains(
                key.clone(),
                channel,
                async { panic!("duplicate database read") },
                async { panic!("unexpected point check") },
            ))
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
        .contains(
            key,
            Uuid::from_u128(281),
            async { panic!("negative membership also comes from the snapshot") },
            async { panic!("unexpected point check") }
        )
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
                    .contains(
                        key,
                        channel,
                        async {
                            let old = snapshot(vec![channel]);
                            entered.send(()).unwrap();
                            released.await.unwrap();
                            Ok(old)
                        },
                        async { panic!("unexpected point check") },
                    )
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
            .contains(
                key.clone(),
                channel,
                async { Ok(snapshot(vec![])) },
                async { panic!("unexpected point check") }
            )
            .await
            .unwrap());
        release.send(()).unwrap();
        assert!(
            old_read.await.unwrap().unwrap(),
            "the original in-flight read may finish"
        );
        assert!(!cache
            .contains(
                key,
                channel,
                async { panic!("new entry was overwritten") },
                async { panic!("unexpected point check") }
            )
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
        .contains(
            key.clone(),
            channel,
            async { Err(DbError::InvalidData("unavailable".into())) },
            async { panic!("unexpected point check") }
        )
        .await
        .is_err());
    assert!(cache
        .contains(
            key.clone(),
            channel,
            async {
                Ok(ReadAt {
                    value: vec![channel],
                    read_at: Instant::now() - MAX_AGE,
                })
            },
            async { panic!("unexpected point check") }
        )
        .await
        .unwrap());
    assert!(!cache
        .contains(
            key.clone(),
            channel,
            async { Ok(snapshot(vec![])) },
            async { panic!("unexpected point check") }
        )
        .await
        .unwrap());
    cache.invalidate(key.0, &key.1);
    assert!(
        cache
            .contains(key, channel, async { Ok(snapshot(vec![channel])) }, async {
                panic!("unexpected point check")
            })
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
                .contains(
                    key,
                    channel,
                    async {
                        entered.send(()).unwrap();
                        std::future::pending().await
                    },
                    async { panic!("unexpected point check") },
                )
                .await
        })
    };
    started.await.unwrap();
    let waiter = cache.contains(key, channel, async { Ok(snapshot(vec![channel])) }, async {
        panic!("unexpected point check")
    });
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
async fn timed_out_waiters_use_point_checks_without_a_second_full_load() {
    let wait = Duration::from_secs(2);
    let cache = MembershipCache::new(wait);
    let key = (community(1), vec![1; 32]);
    let channel = Uuid::from_u128(1);
    let (release, released) = oneshot::channel();
    let mut leader = Box::pin(cache.contains(
        key.clone(),
        channel,
        async {
            released.await.unwrap();
            Ok(snapshot(vec![]))
        },
        async { panic!("leader must load") },
    ));
    assert!(futures_util::poll!(leader.as_mut()).is_pending());
    let start = tokio::time::Instant::now();
    assert!(cache
        .contains(
            key.clone(),
            channel,
            async { panic!("timeout must not start another full-set query") },
            async { Ok(true) }
        )
        .await
        .unwrap());
    assert_eq!(start.elapsed(), wait);
    release.send(()).unwrap();
    assert!(!leader.await.unwrap());
    assert!(!cache
        .contains(
            key,
            channel,
            async { panic!("leader's result must be retained") },
            async { panic!("point result must not overwrite the leader") }
        )
        .await
        .unwrap());
}

#[tokio::test]
async fn cold_users_share_loads_even_when_more_than_sixteen_are_waiting() {
    let cache = MembershipCache::new(Duration::from_secs(5));
    let channel = Uuid::from_u128(1);
    let mut releases = Vec::new();
    let mut loaders = Vec::new();
    let mut waiters = Vec::new();
    for user in 1..=96 {
        let key = (community(1), vec![user; 32]);
        let (release, released) = oneshot::channel();
        releases.push(release);
        loaders.push(Box::pin(cache.contains(
            key.clone(),
            channel,
            async {
                released.await.unwrap();
                Ok(snapshot(vec![channel]))
            },
            async { panic!("other users must not force an uncached point check") },
        )));
        waiters.push(Box::pin(cache.contains(
            key,
            channel,
            async { panic!("same-user checks must share the first load") },
            async { panic!("same-user checks must reuse the snapshot") },
        )));
    }
    for loader in &mut loaders {
        assert!(futures_util::poll!(loader.as_mut()).is_pending());
    }
    for waiter in &mut waiters {
        assert!(futures_util::poll!(waiter.as_mut()).is_pending());
    }
    for release in releases {
        release.send(()).unwrap();
    }
    for loader in loaders {
        assert!(loader.await.unwrap());
    }
    for waiter in waiters {
        assert!(waiter.await.unwrap());
    }
}

#[tokio::test]
async fn per_user_boundary_and_oversize_marker_preserve_authoritative_answers() {
    let cache = MembershipCache::new(Duration::from_secs(1));
    let key = (community(1), vec![1; 32]);
    let ids = (1..=MAX_MEMBERSHIPS as u128)
        .rev()
        .map(Uuid::from_u128)
        .collect::<Vec<_>>();
    let channel = ids[0];
    assert!(cache
        .contains(key.clone(), channel, async { Ok(snapshot(ids)) }, async {
            panic!("exactly the cap fits")
        })
        .await
        .unwrap());
    assert!(!cache
        .contains(
            key.clone(),
            Uuid::nil(),
            async { panic!("complete negative must be cached") },
            async { panic!("no point query") }
        )
        .await
        .unwrap());
    cache.invalidate(key.0, &key.1);
    assert!(
        cache
            .contains(
                key.clone(),
                channel,
                async { Ok(snapshot(vec![Uuid::nil(); MAX_MEMBERSHIPS + 1])) },
                async { Ok(true) }
            )
            .await
            .unwrap(),
        "a truncated list must not deny a valid member"
    );
    for member in [false, true] {
        assert_eq!(
            cache
                .contains(
                    key.clone(),
                    channel,
                    async { panic!("marker must suppress another full-set probe") },
                    async { Ok(member) }
                )
                .await
                .unwrap(),
            member
        );
    }
    assert!(cache
        .contains(
            key.clone(),
            channel,
            async { panic!("marker must suppress another full-set probe") },
            async { Err(DbError::InvalidData("unavailable".into())) }
        )
        .await
        .is_err());
    cache.invalidate(key.0, &key.1);
    assert!(cache
        .contains(key, channel, async { Ok(snapshot(vec![channel])) }, async {
            panic!("invalidation must allow a new snapshot")
        })
        .await
        .unwrap());
}

#[tokio::test]
async fn completed_payload_is_weighed_and_moka_evicts_under_byte_pressure() {
    let key = (community(1), vec![1; 32]);
    let overhead = Entry::weight(&key, &Entry::loading()) as u64;
    let payload = 1000 * std::mem::size_of::<Uuid>() as u64;
    // Enough for two snapshots, independent of entry count.
    let budget = 2 * (overhead + payload);
    let cache = MembershipCache::with_capacity(Duration::from_secs(1), budget);
    let ids = (1..=1000).map(Uuid::from_u128).collect::<Vec<_>>();
    let channel = ids[0];
    let (release, released) = oneshot::channel();
    let mut load = Box::pin(cache.contains(
        key.clone(),
        channel,
        async {
            released.await.unwrap();
            Ok(snapshot(ids.clone()))
        },
        async { panic!("no point check") },
    ));
    assert!(futures_util::poll!(load.as_mut()).is_pending());
    cache.entries.run_pending_tasks();
    assert_eq!(cache.entries.weighted_size(), overhead);
    release.send(()).unwrap();
    assert!(load.await.unwrap());
    cache.entries.run_pending_tasks();
    assert_eq!(
        cache.entries.weighted_size(),
        overhead + payload,
        "completed insertion must reweigh the placeholder"
    );
    let held = cache.entries.get(&key).unwrap();
    let weak = Arc::downgrade(held.snapshot.as_ref().unwrap());
    drop(held);
    for user in 2..=12 {
        assert!(cache
            .contains(
                (community(1), vec![user; 32]),
                channel,
                async { Ok(snapshot(ids.clone())) },
                async { panic!("no point check") }
            )
            .await
            .unwrap());
        cache.entries.run_pending_tasks();
        assert!(cache.entries.weighted_size() <= budget);
    }
    assert!(
        cache.entries.entry_count() <= 2,
        "Moka must evict based on completed payload weights"
    );
    cache.invalidate_community(community(1));
    cache.entries.run_pending_tasks();
    assert_eq!(cache.entries.weighted_size(), 0);
    assert!(weak.upgrade().is_none(), "removed payload must be released");
}

#[tokio::test]
async fn evicting_a_loading_entry_does_not_let_it_overwrite_a_replacement() {
    let cache = MembershipCache::new(Duration::from_secs(1));
    let key = (community(1), vec![1; 32]);
    let channel = Uuid::from_u128(1);
    let (release, released) = oneshot::channel();
    let mut old = Box::pin(cache.contains(
        key.clone(),
        channel,
        async {
            released.await.unwrap();
            Ok(snapshot(vec![channel]))
        },
        async { panic!("no point check") },
    ));
    assert!(futures_util::poll!(old.as_mut()).is_pending());
    cache.entries.invalidate(&key); // Model eviction/expiry independent of membership changes.
    assert!(!cache
        .contains(
            key.clone(),
            channel,
            async { Ok(snapshot(vec![])) },
            async { panic!("no point check") }
        )
        .await
        .unwrap());
    release.send(()).unwrap();
    assert!(old.await.unwrap());
    assert!(!cache
        .contains(
            key,
            channel,
            async { panic!("replacement must remain cached") },
            async { panic!("no point check") }
        )
        .await
        .unwrap());
}

#[tokio::test]
async fn ttl_reclaims_idle_snapshots_and_invalidation_preserves_only_active_readers() {
    let cache = MembershipCache::new(Duration::from_secs(1));
    let key = (community(1), vec![1; 32]);
    let channel = Uuid::from_u128(1);
    assert!(cache
        .contains(
            key.clone(),
            channel,
            async { Ok(snapshot(vec![channel])) },
            async { panic!("no point check") }
        )
        .await
        .unwrap());
    let entry = cache.entries.get(&key).unwrap();
    let weak = Arc::downgrade(entry.snapshot.as_ref().unwrap());
    cache.invalidate(key.0, &key.1);
    cache.entries.run_pending_tasks();
    assert!(
        weak.upgrade().is_some(),
        "an active reader owns the old allocation"
    );
    drop(entry);
    assert!(
        weak.upgrade().is_none(),
        "last reader must release the allocation"
    );
    assert!(cache
        .contains(
            key.clone(),
            channel,
            async { Ok(snapshot(vec![channel])) },
            async { panic!("no point check") }
        )
        .await
        .unwrap());
    let entry = cache.entries.get(&key).unwrap();
    let weak = Arc::downgrade(entry.snapshot.as_ref().unwrap());
    drop(entry);
    tokio::time::sleep(MAX_AGE + Duration::from_millis(50)).await;
    assert!(cache.entries.get(&key).is_none());
    cache.entries.run_pending_tasks();
    assert_eq!(cache.entries.weighted_size(), 0);
    assert!(
        weak.upgrade().is_none(),
        "TTL maintenance must release the idle payload"
    );
}
