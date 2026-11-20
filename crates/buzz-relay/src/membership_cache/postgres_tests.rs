use super::*;
use buzz_pubsub::cache_invalidation::CacheInvalidation;

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires Postgres"]
async fn channel_checks_share_one_database_read_and_follow_membership_changes() {
    let recorder = metrics_util::debugging::DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _recorder = metrics::set_default_local_recorder(&recorder);
    // DebuggingRecorder resets counters to zero on each snapshot.
    let misses_since_last_snapshot = || {
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
    assert_eq!(
        misses_since_last_snapshot(),
        1,
        "only one query may wait for the pool"
    );
    drop(held);
    for read in reads {
        assert!(read.await.unwrap());
    }
    assert_eq!(
        misses_since_last_snapshot(),
        0,
        "the other channel checks must reuse the first read"
    );

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
    assert_eq!(misses_since_last_snapshot(), 1);

    sqlx::query("UPDATE channel_members SET removed_at = NULL WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3")
            .bind(community.as_uuid()).bind(channels[0]).bind(&user).execute(&pool).await.unwrap();
    state.invalidate_membership_local(community, channels[0], &user);
    assert!(state
        .is_member_cached(community, channels[0], &user)
        .await
        .unwrap());
    assert_eq!(misses_since_last_snapshot(), 1);

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
    assert_eq!(misses_since_last_snapshot(), 1);

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

#[tokio::test]
#[ignore = "requires Postgres"]
async fn oversized_membership_reads_are_bounded_and_point_checks_fail_closed() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let state = crate::state::tests::test_state_with_database_pool(pool.clone()).await;
    let community = state
        .db
        .ensure_configured_community(&format!("oversize-{}.local", Uuid::new_v4()))
        .await
        .unwrap()
        .id;
    let user = vec![5u8; 32];
    let channels = (0..MAX_MEMBERSHIPS + 2)
        .map(|_| Uuid::new_v4())
        .collect::<Vec<_>>();
    sqlx::query("INSERT INTO channels (community_id, id, name, visibility, created_by) SELECT $1, id, id::text, 'private', $3 FROM unnest($2::uuid[]) AS id")
        .bind(community.as_uuid()).bind(&channels).bind(&user).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO channel_members (community_id, channel_id, pubkey) SELECT $1, id, $3 FROM unnest($2::uuid[]) AS id")
        .bind(community.as_uuid()).bind(&channels[..MAX_MEMBERSHIPS]).bind(&user).execute(&pool).await.unwrap();
    // The isolated database starts without planner statistics. Refresh them
    // after bulk seeding so a quadratic join plan does not consume the
    // snapshot's freshness window before this cache-boundary assertion.
    sqlx::query("ANALYZE channels")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("ANALYZE channel_members")
        .execute(&pool)
        .await
        .unwrap();
    assert!(state
        .is_member_cached(community, channels[0], &user)
        .await
        .unwrap());
    assert_eq!(
        state
            .membership_cache
            .cached_member(&(community, channels[0], user.clone())),
        Some(true)
    );
    sqlx::query("INSERT INTO channel_members (community_id, channel_id, pubkey) SELECT $1, id, $3 FROM unnest($2::uuid[]) AS id")
        .bind(community.as_uuid()).bind(&channels[MAX_MEMBERSHIPS..]).bind(&user).execute(&pool).await.unwrap();
    state.invalidate_membership_local(community, channels[0], &user);
    let bounded = state
        .db
        .get_active_membership_ids_read_at(community, &user, (MAX_MEMBERSHIPS + 1) as u32)
        .await
        .unwrap();
    assert_eq!(
        bounded.value.len(),
        MAX_MEMBERSHIPS + 1,
        "SQL must not fetch all memberships"
    );
    // Select a channel the truncated query omitted: fallback must still allow it.
    let omitted = *channels
        .iter()
        .find(|id| !bounded.value.contains(id))
        .unwrap();
    assert!(state
        .is_member_cached(community, omitted, &user)
        .await
        .unwrap());
    let key = (community, user.clone());
    assert!(matches!(
        state.membership_cache.entries.get(&key).unwrap().fresh(),
        Some(Memberships::PointChecks)
    ));
    sqlx::query("UPDATE channel_members SET removed_at = NOW() WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3")
        .bind(community.as_uuid()).bind(omitted).bind(&user).execute(&pool).await.unwrap();
    // No invalidation: the strategy marker itself must not cache an allow/deny.
    assert!(!state
        .is_member_cached(community, omitted, &user)
        .await
        .unwrap());
    pool.close().await;
    assert!(state
        .is_member_cached(community, channels[0], &user)
        .await
        .is_err());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn cold_users_wait_for_the_pool_then_reuse_their_snapshots() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let state = crate::state::tests::test_state_with_database_pool(pool.clone()).await;
    let community = state
        .db
        .ensure_configured_community(&format!("cold-users-{}.local", Uuid::new_v4()))
        .await
        .unwrap()
        .id;
    let channel = Uuid::new_v4();
    let users = (1..=96).map(|n| vec![n; 32]).collect::<Vec<_>>();
    sqlx::query("INSERT INTO channels (community_id, id, name, visibility, created_by) VALUES ($1, $2, 'cold-users', 'private', $3)")
        .bind(community.as_uuid()).bind(channel).bind(&users[0]).execute(&pool).await.unwrap();
    for user in &users {
        sqlx::query(
            "INSERT INTO channel_members (community_id, channel_id, pubkey) VALUES ($1, $2, $3)",
        )
        .bind(community.as_uuid())
        .bind(channel)
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    }
    // Force all users to reach the load path before any query can complete.
    let held = pool.acquire().await.unwrap();
    let mut checks = users
        .iter()
        .map(|user| Box::pin(state.is_member_cached(community, channel, user)))
        .collect::<Vec<_>>();
    for check in &mut checks {
        assert!(futures_util::poll!(check.as_mut()).is_pending());
    }
    drop(held);
    for check in checks {
        assert!(check.await.unwrap());
    }
    // With the old semaphore, users beyond its capacity only made point
    // queries. Closing the pool makes their missing cache entries observable.
    pool.close().await;
    for user in users {
        assert!(state
            .is_member_cached(community, channel, &user)
            .await
            .unwrap());
        assert!(!state
            .is_member_cached(community, Uuid::nil(), &user)
            .await
            .unwrap());
    }
}

/// A fan-out's recipients are checked with one bulk read that matches the
/// single-user read: removed memberships and deleted channels do not count,
/// and each user's set is capped so an oversized user takes a point check.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn fan_out_bulk_read_matches_single_user_membership() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let state = crate::state::tests::test_state_with_database_pool(pool.clone()).await;
    let community = state
        .db
        .ensure_configured_community(&format!("bulk-{}.local", Uuid::new_v4()))
        .await
        .unwrap()
        .id;
    let [member, removed, outsider, deleted_only] = [1u8, 2, 3, 4].map(|n| vec![n; 32]);
    let (channel, deleted) = (Uuid::new_v4(), Uuid::new_v4());
    sqlx::query("INSERT INTO channels (community_id, id, name, visibility, created_by) SELECT $1, id, id::text, 'private', $3 FROM unnest($2::uuid[]) AS id")
        .bind(community.as_uuid()).bind([channel, deleted]).bind(&member).execute(&pool).await.unwrap();
    sqlx::query("UPDATE channels SET deleted_at = NOW() WHERE community_id = $1 AND id = $2")
        .bind(community.as_uuid())
        .bind(deleted)
        .execute(&pool)
        .await
        .unwrap();
    for (pubkey, channel_id, removed_at) in [
        (&member, channel, false),
        (&removed, channel, true),
        (&deleted_only, deleted, false),
    ] {
        sqlx::query("INSERT INTO channel_members (community_id, channel_id, pubkey, removed_at) VALUES ($1, $2, $3, CASE WHEN $4 THEN NOW() END)")
            .bind(community.as_uuid()).bind(channel_id).bind(pubkey).bind(removed_at).execute(&pool).await.unwrap();
    }
    let recipients = [&member, &removed, &outsider, &deleted_only];

    let members = state
        .members_cached(community, channel, recipients.map(Vec::clone))
        .await;
    assert_eq!(members.members, HashSet::from([member.clone()]));
    assert!(members.unverified.is_empty());
    for pubkey in recipients {
        assert_eq!(
            state
                .membership_cache
                .cached_member(&(community, channel, pubkey.clone())),
            Some(*pubkey == member),
            "the bulk read is published"
        );
    }

    // Two users above a positive cap each get exactly the cap: a cap on the
    // whole batch would truncate one of them into false denials.
    let heavy = [vec![5u8; 32], vec![6u8; 32]];
    let three = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
    sqlx::query("INSERT INTO channels (community_id, id, name, visibility, created_by) SELECT $1, id, id::text, 'private', $3 FROM unnest($2::uuid[]) AS id")
        .bind(community.as_uuid()).bind(three).bind(&member).execute(&pool).await.unwrap();
    for pubkey in &heavy {
        sqlx::query("INSERT INTO channel_members (community_id, channel_id, pubkey) SELECT $1, id, $3 FROM unnest($2::uuid[]) AS id")
            .bind(community.as_uuid()).bind(three).bind(pubkey).execute(&pool).await.unwrap();
    }
    let capped = state
        .db
        .get_active_membership_ids_bulk_read_at(community, &heavy, 2)
        .await
        .unwrap();
    for pubkey in &heavy {
        let rows = capped.value.iter().filter(|(pk, _)| pk == pubkey).count();
        assert_eq!(rows, 2, "the cap applies per user");
    }

    pool.close().await;
    let uncached = vec![9u8; 32];
    let failed = state
        .members_cached(community, channel, [uncached.clone(), member.clone()])
        .await;
    assert_eq!(failed.members, HashSet::from([member]), "served from cache");
    assert_eq!(
        failed.unverified,
        HashSet::from([uncached]),
        "a failed read is unverified, not denied"
    );
}
