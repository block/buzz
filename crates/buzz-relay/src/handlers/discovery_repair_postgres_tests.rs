//! Postgres-backed tests for republishing channel discovery events (kinds
//! 39000–39002) that a failed post-commit emission left missing.

use std::collections::BTreeMap;
use std::sync::Arc;

use buzz_core::TenantContext;
use nostr::{EventBuilder, Keys, Kind, Tag};
use uuid::Uuid;

use super::discovery_repair::reconcile_missing_channel_discovery_paged;
use super::group_state_postgres_tests::{community, create_channel, http_auth, state};
use crate::handlers::ingest::ingest_event;
use crate::state::AppState;

/// Live relay-signed discovery events of `channel`, by kind, as
/// `(event id, p tags)`.
async fn live_discovery(
    state: &AppState,
    tenant: &TenantContext,
    channel: Uuid,
) -> BTreeMap<i32, (Vec<u8>, Vec<String>)> {
    let rows: Vec<(i32, Vec<u8>, serde_json::Value)> = sqlx::query_as(
        "SELECT kind, id, tags FROM events
         WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3
           AND kind IN (39000, 39001, 39002) AND deleted_at IS NULL",
    )
    .bind(tenant.community().as_uuid())
    .bind(channel)
    .bind(state.relay_keypair.public_key().to_bytes().as_slice())
    .fetch_all(state.db.pool())
    .await
    .expect("live discovery");
    let mut live = BTreeMap::new();
    for (kind, id, tags) in rows {
        let mut members: Vec<String> = tags
            .as_array()
            .expect("tags")
            .iter()
            .filter(|tag| tag[0] == "p")
            .map(|tag| tag[1].as_str().expect("p").to_owned())
            .collect();
        members.sort();
        assert!(
            live.insert(kind, (id, members)).is_none(),
            "two live kind:{kind} events"
        );
    }
    live
}

/// Remove `channel`'s discovery events of `kinds`, as a failed emission
/// leaves them.
async fn drop_discovery(state: &AppState, tenant: &TenantContext, channel: Uuid, kinds: &[i32]) {
    sqlx::query(
        "DELETE FROM events WHERE community_id = $1 AND channel_id = $2 AND kind = ANY($3)",
    )
    .bind(tenant.community().as_uuid())
    .bind(channel)
    .bind(kinds)
    .execute(state.db.pool())
    .await
    .expect("drop discovery");
}

/// Mark `channel`'s discovery events of `kinds` deleted, as a replacement
/// leaves the events it supersedes. They are dated a minute earlier, as a
/// superseded event is: republished in the same second with the same tags,
/// an event would have the deleted one's id.
async fn soft_delete_discovery(
    state: &AppState,
    tenant: &TenantContext,
    channel: Uuid,
    kinds: &[i32],
) {
    sqlx::query(
        "UPDATE events SET deleted_at = NOW(), created_at = created_at - INTERVAL '1 minute'
         WHERE community_id = $1 AND channel_id = $2 AND kind = ANY($3)",
    )
    .bind(tenant.community().as_uuid())
    .bind(channel)
    .bind(kinds)
    .execute(state.db.pool())
    .await
    .expect("soft-delete discovery");
}

/// Date `channel` `minutes` ago, so the sweep's in-flight grace and lookback
/// apply to it.
async fn backdate(state: &AppState, tenant: &TenantContext, channel: Uuid, minutes: i32) {
    sqlx::query(
        "UPDATE channels SET created_at = NOW() - make_interval(mins => $3)
         WHERE community_id = $1 AND id = $2",
    )
    .bind(tenant.community().as_uuid())
    .bind(channel)
    .bind(minutes)
    .execute(state.db.pool())
    .await
    .expect("backdate channel");
}

/// Open the DM of `actor` and `other` with kind:41010.
async fn open_dm(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    actor: &Keys,
    other: &Keys,
) -> (Uuid, bool) {
    let event = EventBuilder::new(Kind::Custom(41010), "")
        .tags([
            Tag::parse(["p", &other.public_key().to_hex()]).expect("p"),
            // A unique tag so identical opens are distinct events.
            Tag::parse(["client", &Uuid::new_v4().to_string()]).expect("client"),
        ])
        .sign_with_keys(actor)
        .expect("sign 41010");
    let result = ingest_event(state, tenant, event, http_auth(actor))
        .await
        .unwrap_or_else(|error| panic!("open DM: {error:?}"));
    let response: serde_json::Value =
        serde_json::from_str(result.message.strip_prefix("response:").expect("response"))
            .expect("response json");
    (
        response["channel_id"]
            .as_str()
            .expect("channel_id")
            .parse()
            .expect("uuid"),
        response["created"].as_bool().expect("created"),
    )
}

/// Reopening a DM whose roster is missing publishes it again, with both
/// people, and a reopen of a healthy DM publishes nothing.
/// Mutations: drop the reopen repair → no live kind:39002 → RED; republish
/// unconditionally → the healthy reopen replaces kind:39000 → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn reopening_a_dm_republishes_its_missing_roster() {
    let state = state().await;
    let tenant = community(&state, "discovery-reopen").await;
    let alice = Keys::generate();
    let bob = Keys::generate();
    let mut people = vec![alice.public_key().to_hex(), bob.public_key().to_hex()];
    people.sort();

    let (dm, created) = open_dm(&state, &tenant, &alice, &bob).await;
    assert!(created, "the first open creates the DM");
    let healthy = live_discovery(&state, &tenant, dm).await;
    assert_eq!(
        healthy.keys().copied().collect::<Vec<_>>(),
        [39000, 39001, 39002]
    );

    // A healthy reopen writes nothing.
    let (reopened, created) = open_dm(&state, &tenant, &bob, &alice).await;
    assert_eq!((reopened, created), (dm, false));
    assert_eq!(live_discovery(&state, &tenant, dm).await, healthy);

    // The roster is lost, as when emission fails after kind:39000.
    drop_discovery(&state, &tenant, dm, &[39002]).await;
    assert!(!live_discovery(&state, &tenant, dm)
        .await
        .contains_key(&39002));

    let (reopened, created) = open_dm(&state, &tenant, &bob, &alice).await;
    assert_eq!((reopened, created), (dm, false));
    let repaired = live_discovery(&state, &tenant, dm).await;
    let (_, members) = repaired
        .get(&39002)
        .expect("the reopen republishes the roster");
    assert_eq!(members, &people, "the roster names both people");
    assert!(repaired.contains_key(&39000) && repaired.contains_key(&39001));
}

/// The sweep republishes every channel in its window that lacks any
/// discovery kind, across pages, and leaves healthy, too-old and in-flight
/// channels alone.
/// Mutations: check only kind:39000, or count deleted events as live → the
/// partial channel stays without a roster → RED; stop after the first page →
/// a later channel stays broken → RED; start the scan before the lookback or
/// drop the grace bound → the old or the in-flight channel is republished →
/// RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn the_sweep_republishes_recent_channels_missing_any_discovery_kind() {
    let state = state().await;
    let tenant = community(&state, "discovery-sweep").await;
    let owner = Keys::generate();

    let partial = create_channel(&state, &tenant, &owner, &[]).await;
    let bare = create_channel(&state, &tenant, &owner, &[]).await;
    let healthy = create_channel(&state, &tenant, &owner, &[]).await;
    let too_old = create_channel(&state, &tenant, &owner, &[]).await;
    let in_flight = create_channel(&state, &tenant, &owner, &[]).await;
    for channel in [partial, bare, healthy] {
        backdate(&state, &tenant, channel, 10).await;
    }
    backdate(&state, &tenant, too_old, 120).await;

    // Deleted rows don't count as live.
    soft_delete_discovery(&state, &tenant, partial, &[39001, 39002]).await;
    drop_discovery(&state, &tenant, bare, &[39000, 39001, 39002]).await;
    drop_discovery(&state, &tenant, too_old, &[39002]).await;
    drop_discovery(&state, &tenant, in_flight, &[39002]).await;
    let healthy_before = live_discovery(&state, &tenant, healthy).await;

    // One channel per page, so each repaired channel is on its own page.
    let summary =
        reconcile_missing_channel_discovery_paged(&state, chrono::Duration::minutes(30), 1)
            .await
            .expect("sweep");
    assert!(summary.repaired >= 2, "{summary:?}");

    let all = [39000, 39001, 39002];
    for channel in [partial, bare] {
        let live = live_discovery(&state, &tenant, channel).await;
        assert_eq!(live.keys().copied().collect::<Vec<_>>(), all, "{channel}");
        assert_eq!(
            live[&39002].1,
            vec![owner.public_key().to_hex()],
            "the roster names the owner"
        );
    }
    assert_eq!(
        live_discovery(&state, &tenant, healthy).await,
        healthy_before
    );
    for channel in [too_old, in_flight] {
        assert!(
            !live_discovery(&state, &tenant, channel)
                .await
                .contains_key(&39002),
            "{channel} is outside the window"
        );
    }
}
