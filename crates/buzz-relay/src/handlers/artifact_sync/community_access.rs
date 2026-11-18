//! One jittered node-wide backstop for lost admission invalidations. Queries
//! batch distinct active principals, not subscriptions. Content delivery never
//! uses the result as a cache: it authorizes again through its queue guard.
use super::*;

pub(super) async fn revalidate(state: &AppState) {
    type Readers = HashMap<
        (Vec<u8>, Option<Vec<u8>>, bool, bool),
        (CommunityArtifactPrincipal, Vec<Arc<SyncHandle>>),
    >;
    let mut tenants: HashMap<CommunityId, Readers> = HashMap::new();
    for entry in state.artifact_syncs.syncs.iter() {
        let Some(interest) = &entry.community else {
            continue;
        };
        if !interest
            .connection
            .upgrade()
            .is_some_and(|conn| community_credential_current(state, &conn, &interest.principal))
        {
            entry.resync("resync: community credential changed");
            continue;
        }
        let (_, handles) = tenants
            .entry(interest.tenant)
            .or_default()
            .entry((
                interest.principal.pubkey.clone(),
                interest.principal.verified_owner.clone(),
                interest.principal.require_membership,
                interest.principal.owner_admission,
            ))
            .or_insert_with(|| (interest.principal.clone(), vec![]));
        handles.push(Arc::clone(entry.value()));
    }
    for (community, readers) in tenants {
        let readers: Vec<_> = readers.into_values().collect();
        for batch in readers.chunks(256) {
            let principals: Vec<_> = batch.iter().map(|(p, _)| p.clone()).collect();
            let allowed = state
                .db
                .community_artifact_admission_batch(community, &principals)
                .await;
            for (index, (_, handles)) in batch.iter().enumerate() {
                let current = allowed.as_ref().ok().and_then(|v| v.get(index));
                for handle in handles {
                    let permitted = current.is_some_and(|(required, admitted)| {
                        *admitted
                            && handle
                                .community
                                .as_ref()
                                .and_then(|i| i.connection.upgrade())
                                .is_some_and(|conn| {
                                    community_policy_current(state, &conn, *required)
                                })
                    });
                    if !permitted {
                        handle.resync("resync: community access changed");
                    }
                }
            }
        }
    }
}

/// The process has one watcher, with 45–75 s jitter. Immediate invalidations
/// normally close syncs; this bounds a lost cross-pod revocation while idle.
pub async fn watch(state: Arc<AppState>) {
    loop {
        let delay = 45 + rand::random::<u64>() % 31;
        tokio::time::sleep(Duration::from_secs(delay)).await;
        revalidate(&state).await;
    }
}
