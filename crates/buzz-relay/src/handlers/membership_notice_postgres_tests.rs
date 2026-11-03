//! Postgres-backed tests for the membership notices sent by a channel delete,
//! a moderation kick and a moderation DM: each notice reaches the member's
//! live subscription, and a channel-list read made on receipt already shows
//! the change. "On receipt" is enforced by `notice_barrier`, which parks the
//! producer at the notice while the test reads.

use std::sync::Arc;

use buzz_core::kind::{
    KIND_DM_HIDE, KIND_DM_VISIBILITY, KIND_MEMBER_ADDED_NOTIFICATION,
    KIND_MEMBER_REMOVED_NOTIFICATION,
};
use buzz_core::TenantContext;
use nostr::{EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};
use tokio::sync::mpsc::Receiver;
use uuid::Uuid;

use super::group_state_postgres_tests::{create_channel, http_auth, test_conn};
use super::side_effects::notice_barrier;
use crate::handlers::ingest::ingest_event;
use crate::state::AppState;

async fn fixture() -> (Arc<AppState>, TenantContext) {
    let state = crate::api::bridge::postgres_tests::bridge_handler_test_state()
        .await
        .expect("local Postgres and Redis");
    let host = format!("membership-notices-{}.local", Uuid::new_v4().simple());
    let community = state
        .db
        .ensure_configured_community(&host)
        .await
        .expect("ensure community")
        .id;
    (state, TenantContext::resolved(community, &host))
}

async fn publish(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    keys: &Keys,
    kind: u16,
    tags: &[&[&str]],
) {
    let tags: Vec<Tag> = tags
        .iter()
        .map(|tag| Tag::parse(tag.iter().copied()).unwrap())
        .collect();
    let event = EventBuilder::new(Kind::Custom(kind), "")
        .tags(tags)
        .sign_with_keys(keys)
        .expect("sign");
    let result = ingest_event(state, tenant, event, http_auth(keys))
        .await
        .unwrap_or_else(|error| panic!("ingest kind:{kind}: {error:?}"));
    assert!(result.accepted, "kind:{kind} refused: {}", result.message);
}

/// Register `reader` as a connected client holding the global notice
/// subscription clients send, and return its outbound frames.
fn subscribe_notices(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    reader: &Keys,
) -> Receiver<axum::extract::ws::Message> {
    let conn_id = Uuid::new_v4();
    let (tx, rx) = tokio::sync::mpsc::channel(32);
    state.conn_manager.register(
        conn_id,
        tx,
        tokio::sync::mpsc::channel(4).0,
        tokio::sync::mpsc::channel(1).0,
        None,
        tokio_util::sync::CancellationToken::new(),
        tenant.community(),
        Arc::new(std::sync::atomic::AtomicU8::new(0)),
        Arc::new(tokio::sync::Mutex::new(Default::default())),
        3,
        crate::state::CommunityConnectionControl::new(tokio_util::sync::CancellationToken::new()),
    );
    state
        .conn_manager
        .set_authenticated_pubkey(conn_id, reader.public_key().to_bytes().to_vec());
    let filter = json!({
        "kinds": [KIND_MEMBER_ADDED_NOTIFICATION, KIND_MEMBER_REMOVED_NOTIFICATION],
        "#p": [reader.public_key().to_hex()],
    });
    state.sub_registry.register_scoped(
        tenant.community(),
        conn_id,
        "notices".into(),
        vec![
            serde_json::from_value(filter).expect("filter"),
            serde_json::from_value(json!({
                "kinds": [KIND_DM_VISIBILITY],
                "#p": [reader.public_key().to_hex()],
            }))
            .expect("filter"),
        ],
        None,
    );
    rx
}

/// The `(kind, h tags)` of every live event delivered so far.
fn live_events(rx: &mut Receiver<axum::extract::ws::Message>) -> Vec<(u64, Vec<String>)> {
    let mut events = Vec::new();
    while let Ok(frame) = rx.try_recv() {
        let axum::extract::ws::Message::Text(text) = frame else {
            continue;
        };
        let message: Value = serde_json::from_str(&text).expect("relay frame");
        if message[0] == "EVENT" {
            events.push((
                message[2]["kind"].as_u64().unwrap(),
                tag_values(&message[2], "h"),
            ));
        }
    }
    events
}

fn notices(rx: &mut Receiver<axum::extract::ws::Message>) -> Vec<(u64, String)> {
    live_events(rx)
        .into_iter()
        .filter(|(kind, _)| *kind != KIND_DM_VISIBILITY as u64)
        .map(|(kind, h)| (kind, h.concat()))
        .collect()
}

fn tag_values(event: &Value, name: &str) -> Vec<String> {
    event["tags"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|tag| tag[0] == name)
        .map(|tag| tag[1].as_str().unwrap().to_owned())
        .collect()
}

/// The `name` tag values of the events a one-shot read as `reader` returns.
async fn read_tags(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    reader: &Keys,
    filter: Value,
    name: &str,
) -> Vec<String> {
    let (conn, mut rx) = test_conn(tenant, reader);
    let filters: Vec<nostr::Filter> = serde_json::from_value(json!([filter])).unwrap();
    crate::handlers::req::handle_req("read".into(), filters, vec![None], conn, state.clone()).await;
    let mut values = Vec::new();
    while let Ok(frame) = rx.try_recv() {
        let axum::extract::ws::Message::Text(text) = frame else {
            continue;
        };
        let message: Value = serde_json::from_str(&text).expect("relay frame");
        match message[0].as_str() {
            Some("EVENT") => values.extend(tag_values(&message[2], name)),
            Some("EOSE") | Some("CLOSED") => return values,
            _ => {}
        }
    }
    panic!("no EOSE");
}

/// A channel-list read as `reader`: the channels its 39002 discovery names,
/// as a client builds its sidebar.
async fn listed_channels(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    reader: &Keys,
) -> Vec<String> {
    let filter = json!({"kinds": [39002], "#p": [reader.public_key().to_hex()]});
    read_tags(state, tenant, reader, filter, "d").await
}

/// The DMs `reader`'s current NIP-DV snapshot hides from its sidebar.
async fn hidden_dms(state: &Arc<AppState>, tenant: &TenantContext, reader: &Keys) -> Vec<String> {
    let filter = json!({"kinds": [KIND_DM_VISIBILITY], "#p": [reader.public_key().to_hex()]});
    read_tags(state, tenant, reader, filter, "h").await
}

/// Wait for the producer to reach its next notice to the parked reader, run
/// `on_receipt` while it is held there, then let it continue. Returns the
/// notice's channel and what `on_receipt` saw.
async fn at_receipt<T>(
    barrier: &mut tokio::sync::mpsc::UnboundedReceiver<notice_barrier::Parked>,
    on_receipt: impl AsyncFnOnce(String) -> T,
) -> (String, T) {
    let (channel, release) =
        tokio::time::timeout(std::time::Duration::from_secs(10), barrier.recv())
            .await
            .expect("producer reached no notice")
            .expect("barrier open");
    let seen = on_receipt(channel.to_string()).await;
    release.send(()).expect("producer waiting");
    (channel.to_string(), seen)
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn channel_delete_notifies_every_member_after_cleanup() {
    let (state, tenant) = fixture().await;
    let owner = Keys::generate();
    let member = Keys::generate();
    let channel = create_channel(&state, &tenant, &owner, &[]).await;
    let h = channel.to_string();
    publish(
        &state,
        &tenant,
        &owner,
        9000,
        &[&["h", &h], &["p", &member.public_key().to_hex()]],
    )
    .await;
    assert!(listed_channels(&state, &tenant, &member).await.contains(&h));
    let mut owner_rx = subscribe_notices(&state, &tenant, &owner);
    let mut member_rx = subscribe_notices(&state, &tenant, &member);
    let mut barrier = notice_barrier::install(&member.public_key().to_bytes());

    let delete = tokio::spawn({
        let (state, tenant, owner, h) = (state.clone(), tenant.clone(), owner.clone(), h.clone());
        async move { publish(&state, &tenant, &owner, 9008, &[&["h", &h]]).await }
    });
    let (noticed, listed) = at_receipt(&mut barrier, async |_| {
        listed_channels(&state, &tenant, &member).await
    })
    .await;
    delete.await.expect("delete");

    assert_eq!(noticed, h);
    assert!(!listed.contains(&h), "notice sent before discovery cleanup");
    for rx in [&mut owner_rx, &mut member_rx] {
        assert_eq!(
            notices(rx),
            vec![(KIND_MEMBER_REMOVED_NOTIFICATION as u64, h.clone())]
        );
    }
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn moderation_kick_notifies_the_kicked_member_after_cleanup() {
    let (state, tenant) = fixture().await;
    let owner = Keys::generate();
    let member = Keys::generate();
    let channel = create_channel(&state, &tenant, &owner, &[]).await;
    let h = channel.to_string();
    let member_bytes = member.public_key().to_bytes();
    publish(
        &state,
        &tenant,
        &owner,
        9000,
        &[&["h", &h], &["p", &member.public_key().to_hex()]],
    )
    .await;
    assert!(listed_channels(&state, &tenant, &member).await.contains(&h));
    let mut rx = subscribe_notices(&state, &tenant, &member);
    let mut barrier = notice_barrier::install(&member_bytes);

    // The kick mutation commits the removal; live effects follow.
    state
        .db
        .remove_member(
            tenant.community(),
            channel,
            &member_bytes,
            &owner.public_key().to_bytes(),
        )
        .await
        .expect("remove member");
    let effects = tokio::spawn({
        let (state, tenant) = (state.clone(), tenant.clone());
        async move {
            crate::handlers::side_effects::apply_kick_live_side_effects(
                &tenant,
                &state,
                channel,
                &member_bytes,
            )
            .await
        }
    });
    let (noticed, listed) = at_receipt(&mut barrier, async |_| {
        listed_channels(&state, &tenant, &member).await
    })
    .await;
    effects.await.expect("task").expect("kick live effects");

    assert_eq!(noticed, h);
    assert!(
        !listed.contains(&h),
        "notice sent before the roster refresh"
    );
    assert_eq!(
        notices(&mut rx),
        vec![(KIND_MEMBER_REMOVED_NOTIFICATION as u64, h.clone())]
    );
}

/// A moderation DM resurfaces the DM, and on receipt of its notice the
/// channel list shows it and the NIP-DV snapshot no longer hides it. The
/// second delivery follows a real hide without waiting a second: its notice
/// may coalesce with the first, but the live snapshot still unhides it.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn moderation_dm_notifies_the_recipient_once_visible() {
    let (state, tenant) = fixture().await;
    let recipient = Keys::generate();
    let recipient_bytes = recipient.public_key().to_bytes();
    let mut rx = subscribe_notices(&state, &tenant, &recipient);
    let mut barrier = notice_barrier::install(&recipient_bytes);
    let deliver = || {
        let (state, tenant) = (state.clone(), tenant.clone());
        tokio::spawn(async move {
            crate::handlers::moderation_notices::send_moderation_notice(
                &tenant,
                &state,
                &recipient_bytes,
                crate::handlers::moderation_notices::ModerationNotice::ContentActioned {
                    action_id: Uuid::new_v4(),
                    public_reason: "spam".into(),
                },
                chrono::Utc::now(),
            )
            .await
        })
    };
    let on_receipt = async |dm: String| {
        (
            listed_channels(&state, &tenant, &recipient)
                .await
                .contains(&dm),
            hidden_dms(&state, &tenant, &recipient).await.contains(&dm),
        )
    };

    let first = deliver();
    let (dm, seen) = at_receipt(&mut barrier, on_receipt).await;
    first.await.expect("task").expect("first notice");
    assert_eq!(seen, (true, false), "listed and visible on receipt");
    assert_eq!(
        notices(&mut rx),
        vec![(KIND_MEMBER_ADDED_NOTIFICATION as u64, dm.clone())]
    );

    publish(
        &state,
        &tenant,
        &recipient,
        KIND_DM_HIDE as u16,
        &[&["h", &dm]],
    )
    .await;
    assert!(hidden_dms(&state, &tenant, &recipient).await.contains(&dm));
    live_events(&mut rx);

    let second = deliver();
    let (again, seen) = at_receipt(&mut barrier, on_receipt).await;
    second.await.expect("task").expect("second notice");
    assert_eq!(again, dm);
    assert_eq!(seen, (true, false), "listed and visible on receipt");
    let latest_snapshot = live_events(&mut rx)
        .into_iter()
        .rev()
        .find(|(kind, _)| *kind == KIND_DM_VISIBILITY as u64)
        .expect("live visibility snapshot");
    assert!(
        !latest_snapshot.1.contains(&dm),
        "client still hides the DM"
    );
}
