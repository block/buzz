//! Membership discovery must not cache a snapshot from before invalidation.

use super::*;
use buzz_db::channel_members::ChannelSet;

type Key = (CommunityId, Vec<u8>);

#[derive(Clone, Copy, Debug)]
enum Route {
    MembershipLocal,
    MembershipPublish,
    MembershipReceived,
    AccessibleLocal,
    AccessiblePublish,
    AccessibleReceived,
    DeletedLocal,
    DeletedPublish,
    DeletedReceived,
}

impl Route {
    fn apply(self, state: &AppState, community: CommunityId, channel: Uuid, reader: &[u8]) {
        let tenant = TenantContext::resolved(community, "cache.example.org");
        match self {
            Self::MembershipLocal => state.invalidate_membership_local(community, channel, reader),
            Self::MembershipPublish => state.invalidate_membership(&tenant, channel, reader),
            Self::MembershipReceived => state.apply_cache_invalidation(
                community,
                CacheInvalidation::Membership {
                    channel_id: channel,
                    pubkey: reader.to_vec(),
                },
            ),
            Self::AccessibleLocal => state.invalidate_all_accessible_channels_local(community),
            Self::AccessiblePublish => state.invalidate_all_accessible_channels(&tenant),
            Self::AccessibleReceived => {
                state.apply_cache_invalidation(community, CacheInvalidation::AccessibleAll)
            }
            Self::DeletedLocal => state.invalidate_channel_deleted_local(community),
            Self::DeletedPublish => state.invalidate_channel_deleted(&tenant),
            Self::DeletedReceived => {
                state.apply_cache_invalidation(community, CacheInvalidation::ChannelDeleted)
            }
        }
    }

    fn reader_only(self) -> bool {
        matches!(
            self,
            Self::MembershipLocal | Self::MembershipPublish | Self::MembershipReceived
        )
    }
}

#[derive(Clone, Copy, Debug)]
enum Replacement {
    NotStarted,
    Completed,
    Reading,
}

fn cached(state: &AppState, key: &Key) -> Option<ChannelSet> {
    state
        .member_channels_cache
        .get(key)
        .map(|member| (*member.set).clone())
}

/// Pause a database snapshot after its connection checkout, before completion.
async fn held_read(
    released: tokio::sync::oneshot::Receiver<()>,
    set: ChannelSet,
) -> Result<buzz_db::ReadAt<ChannelSet>, buzz_db::DbError> {
    let snapshot = read_now(set);
    released.await.expect("release the read");
    Ok(snapshot)
}

async fn check_routes(replacement: Replacement) {
    let state = test_state().await;
    for route in [
        Route::MembershipLocal,
        Route::MembershipPublish,
        Route::MembershipReceived,
        Route::AccessibleLocal,
        Route::AccessiblePublish,
        Route::AccessibleReceived,
        Route::DeletedLocal,
        Route::DeletedPublish,
        Route::DeletedReceived,
    ] {
        for joined in [false, true] {
            let community = CommunityId::from_uuid(Uuid::new_v4());
            let neighbor = CommunityId::from_uuid(Uuid::new_v4());
            let reader = vec![1; 32];
            let key = (community, reader.clone());
            let stream = Uuid::new_v4();
            let channels = ChannelSet {
                implicit: vec![stream],
                named_only: vec![(Uuid::new_v4(), "system".to_owned())],
            };
            let empty = ChannelSet::default();
            let (stale, fresh) = if joined {
                (&empty, &channels)
            } else {
                (&channels, &empty)
            };
            let context = format!("{route:?}, {replacement:?}, joined={joined}");

            // Neither the cached nor in-flight sets of another community may
            // be disturbed. A per-reader drop also spares other local readers.
            let cached_peer = (community, vec![2; 32]);
            let cached_neighbor = (neighbor, reader.clone());
            state.seed_member_channels(cached_peer.clone(), channels.clone());
            state.seed_member_channels(cached_neighbor.clone(), channels.clone());
            let loading_peer = (community, vec![3; 32]);
            let loading_neighbor = (neighbor, vec![3; 32]);
            let (release_peer, peer_released) = tokio::sync::oneshot::channel();
            let mut peer = Box::pin(state.member_channels(
                loading_peer.clone(),
                held_read(peer_released, channels.clone()),
            ));
            let (release_neighbor, neighbor_released) = tokio::sync::oneshot::channel();
            let mut other_community = Box::pin(state.member_channels(
                loading_neighbor.clone(),
                held_read(neighbor_released, channels.clone()),
            ));
            assert!(futures_util::poll!(peer.as_mut()).is_pending());
            assert!(futures_util::poll!(other_community.as_mut()).is_pending());

            let (release_old, old_released) = tokio::sync::oneshot::channel();
            let mut old = Box::pin(
                state.member_channels(key.clone(), held_read(old_released, stale.clone())),
            );
            assert!(futures_util::poll!(old.as_mut()).is_pending());
            route.apply(&state, community, stream, &reader);
            assert_eq!(cached(&state, &key), None, "{context}");
            assert_eq!(
                cached(&state, &cached_peer),
                route.reader_only().then(|| channels.clone()),
                "{context}"
            );
            assert_eq!(
                cached(&state, &cached_neighbor),
                Some(channels.clone()),
                "{context}"
            );

            if matches!(replacement, Replacement::Completed) {
                let answer = state
                    .member_channels(key.clone(), async { Ok(read_now(fresh.clone())) })
                    .await
                    .unwrap();
                assert_eq!(*answer, *fresh, "{context}");
            }
            let (release_new, new_released) = tokio::sync::oneshot::channel();
            let mut new = Box::pin(
                state.member_channels(key.clone(), held_read(new_released, fresh.clone())),
            );
            if matches!(replacement, Replacement::Reading) {
                assert!(futures_util::poll!(new.as_mut()).is_pending());
                assert!(state.member_channels_loads.is_loading(&key), "{context}");
            }

            release_old.send(()).unwrap();
            assert_eq!(
                *old.await.unwrap(),
                *stale,
                "{context}: old caller keeps its snapshot"
            );
            assert_eq!(
                cached(&state, &key),
                matches!(replacement, Replacement::Completed).then(|| fresh.clone()),
                "{context}: old read must neither refill nor overwrite"
            );
            if matches!(replacement, Replacement::Reading) {
                assert!(
                    state.member_channels_loads.is_loading(&key),
                    "{context}: old completion must not consume the new token"
                );
            }
            release_new.send(()).unwrap();
            assert_eq!(*new.await.unwrap(), *fresh, "{context}");
            assert_eq!(
                cached(&state, &key),
                Some(fresh.clone()),
                "{context}: new snapshot is cached"
            );
            let hit = state
                .member_channels(key.clone(), async { panic!("a fresh hit must not read") })
                .await
                .unwrap();
            assert_eq!(*hit, *fresh, "{context}");

            release_peer.send(()).unwrap();
            release_neighbor.send(()).unwrap();
            assert_eq!(*peer.await.unwrap(), channels, "{context}");
            assert_eq!(*other_community.await.unwrap(), channels, "{context}");
            assert_eq!(
                cached(&state, &loading_peer),
                route.reader_only().then(|| channels.clone()),
                "{context}: in-flight reader scope"
            );
            assert_eq!(
                cached(&state, &loading_neighbor),
                Some(channels),
                "{context}: in-flight community scope"
            );
        }
    }
}

/// A read predating any invalidation route may answer its own caller, but
/// cannot refill the dropped set. Covers stale grants and stale denials for
/// both implicit and named-only channels, plus reader/community scoping.
/// Mutation: publish without checking the load token -> RED.
#[tokio::test]
async fn every_route_fences_a_fill_before_the_replacement_starts() {
    check_routes(Replacement::NotStarted).await;
}

/// A pre-invalidation read completing last must not overwrite a new snapshot.
/// Mutation: always run `publish` even when its token is no longer current -> RED.
#[tokio::test]
async fn every_route_preserves_a_completed_replacement() {
    check_routes(Replacement::Completed).await;
}

/// A new read has started but not published when the old read completes.
/// The old completion must neither fill the current generation nor consume
/// its token; the new read alone must cache the post-invalidation snapshot.
/// Mutation: publish using whichever token is current for the key -> RED.
#[tokio::test]
async fn every_route_preserves_a_still_reading_replacement() {
    check_routes(Replacement::Reading).await;
}
