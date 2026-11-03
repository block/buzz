//! Postgres-backed tests for the membership notices sent by a channel delete,
//! a moderation kick and a moderation DM: each notice reaches the member's
//! live subscription, and a channel-list read made on receipt already shows
//! the change.

use std::sync::Arc;

use buzz_core::kind::{KIND_MEMBER_ADDED_NOTIFICATION, KIND_MEMBER_REMOVED_NOTIFICATION};
use buzz_core::TenantContext;
use nostr::{EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};
use tokio::sync::mpsc::Receiver;
use uuid::Uuid;

use super::group_state_postgres_tests::{create_channel, http_auth, test_conn};
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
        vec![serde_json::from_value(filter).expect("filter")],
        None,
    );
    rx
}

/// The `(kind, h)` of every live notice delivered so far.
fn live_notices(rx: &mut Receiver<axum::extract::ws::Message>) -> Vec<(u64, String)> {
    let mut notices = Vec::new();
    while let Ok(frame) = rx.try_recv() {
        let axum::extract::ws::Message::Text(text) = frame else {
            continue;
        };
        let message: Value = serde_json::from_str(&text).expect("relay frame");
        if message[0] != "EVENT" {
            continue;
        }
        let event = &message[2];
        let h = event["tags"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tag| tag[0] == "h")
            .map(|tag| tag[1].as_str().unwrap().to_owned())
            .unwrap_or_default();
        notices.push((event["kind"].as_u64().unwrap(), h));
    }
    notices
}

/// A channel-list read as `reader`: the channels its 39000/39002 discovery
/// names, as a client builds its sidebar.
async fn listed_channels(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    reader: &Keys,
) -> Vec<String> {
    let (conn, mut rx) = test_conn(tenant, reader);
    let filters: Vec<nostr::Filter> = serde_json::from_value(json!([
        {"kinds": [39002], "#p": [reader.public_key().to_hex()]},
    ]))
    .unwrap();
    crate::handlers::req::handle_req("list".into(), filters, vec![None], conn, state.clone()).await;
    let mut channels = Vec::new();
    while let Ok(frame) = rx.try_recv() {
        let axum::extract::ws::Message::Text(text) = frame else {
            continue;
        };
        let message: Value = serde_json::from_str(&text).expect("relay frame");
        match message[0].as_str() {
            Some("EVENT") => {
                for tag in message[2]["tags"].as_array().unwrap() {
                    if tag[0] == "d" {
                        channels.push(tag[1].as_str().unwrap().to_owned());
                    }
                }
            }
            Some("EOSE") | Some("CLOSED") => return channels,
            _ => {}
        }
    }
    panic!("no EOSE");
}

/// Mutation: drop the notice loop in `handle_delete_group` → RED; emit the
/// notices before `soft_delete_discovery_events` → the listed check is RED.
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

    publish(&state, &tenant, &owner, 9008, &[&["h", &h]]).await;

    for (reader, rx) in [(&owner, &mut owner_rx), (&member, &mut member_rx)] {
        assert_eq!(
            live_notices(rx),
            vec![(KIND_MEMBER_REMOVED_NOTIFICATION as u64, h.clone())]
        );
        assert!(!listed_channels(&state, &tenant, reader).await.contains(&h));
    }
}

/// Mutation: drop the notice or the discovery refresh in
/// `apply_kick_live_side_effects` → RED.
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
    crate::handlers::side_effects::apply_kick_live_side_effects(
        &tenant,
        &state,
        channel,
        &member_bytes,
    )
    .await
    .expect("kick live effects");

    assert_eq!(
        live_notices(&mut rx),
        vec![(KIND_MEMBER_REMOVED_NOTIFICATION as u64, h.clone())]
    );
    assert!(!listed_channels(&state, &tenant, &member).await.contains(&h));
}

/// Mutation: drop the notice in `send_moderation_notice` → RED. A second
/// delivery after the recipient hid the DM notifies again.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn moderation_dm_notifies_the_recipient_once_listed() {
    let (state, tenant) = fixture().await;
    let recipient = Keys::generate();
    let recipient_bytes = recipient.public_key().to_bytes();
    let mut rx = subscribe_notices(&state, &tenant, &recipient);
    let notice = || crate::handlers::moderation_notices::ModerationNotice::ContentActioned {
        action_id: Uuid::new_v4(),
        public_reason: "spam".into(),
    };

    crate::handlers::moderation_notices::send_moderation_notice(
        &tenant,
        &state,
        &recipient_bytes,
        notice(),
        chrono::Utc::now(),
    )
    .await
    .expect("first notice");
    let notices = live_notices(&mut rx);
    assert_eq!(notices.len(), 1, "{notices:?}");
    let (kind, dm) = notices[0].clone();
    assert_eq!(kind, KIND_MEMBER_ADDED_NOTIFICATION as u64);
    assert!(listed_channels(&state, &tenant, &recipient)
        .await
        .contains(&dm));

    let dm_id: Uuid = dm.parse().unwrap();
    state
        .db
        .hide_dm(tenant.community(), dm_id, &recipient_bytes)
        .await
        .expect("hide dm");
    // Notices are deterministic per second; a later delivery is a new event.
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    crate::handlers::moderation_notices::send_moderation_notice(
        &tenant,
        &state,
        &recipient_bytes,
        notice(),
        chrono::Utc::now(),
    )
    .await
    .expect("second notice");
    assert_eq!(
        live_notices(&mut rx),
        vec![(KIND_MEMBER_ADDED_NOTIFICATION as u64, dm.clone())]
    );
    assert!(listed_channels(&state, &tenant, &recipient)
        .await
        .contains(&dm));
}
