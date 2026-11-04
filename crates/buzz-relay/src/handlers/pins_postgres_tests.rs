//! Postgres-backed tests for kind:40004 pinned messages: who may pin, what a
//! pin may name, and how a pin is removed.

use std::sync::Arc;

use buzz_core::kind::KIND_STREAM_MESSAGE_PINNED;
use buzz_core::TenantContext;
use buzz_db::event::EventQuery;
use nostr::{Event, EventBuilder, EventId, Keys, Kind, Tag};
use uuid::Uuid;

use crate::handlers::group_state_postgres_tests::{community, create_channel, http_auth, state};
use crate::handlers::ingest::{ingest_event, IngestError};
use crate::state::AppState;

fn event(keys: &Keys, kind: u16, content: &str, tags: &[&[&str]]) -> Event {
    EventBuilder::new(Kind::Custom(kind), content)
        .tags(tags.iter().map(|t| Tag::parse(t.iter().copied()).unwrap()))
        .allow_self_tagging()
        .sign_with_keys(keys)
        .expect("sign")
}

async fn send(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    keys: &Keys,
    event: Event,
) -> Result<(), IngestError> {
    let result = ingest_event(state, tenant, event, http_auth(keys)).await?;
    assert!(result.accepted, "{}", result.message);
    Ok(())
}

async fn post(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    keys: &Keys,
    channel: Uuid,
    kind: u16,
) -> EventId {
    let ev = event(keys, kind, "hello", &[&["h", &channel.to_string()]]);
    let id = ev.id;
    send(state, tenant, keys, ev).await.expect("post message");
    id
}

async fn add(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    owner: &Keys,
    channel: Uuid,
    who: &Keys,
    role: &str,
) {
    let ev = event(
        owner,
        9000,
        "",
        &[
            &["h", &channel.to_string()],
            &["p", &who.public_key().to_hex(), role],
        ],
    );
    send(state, tenant, owner, ev).await.expect("add member");
}

fn pin(keys: &Keys, channel: Uuid, target: EventId) -> Event {
    event(
        keys,
        40004,
        "",
        &[&["h", &channel.to_string()], &["e", &target.to_hex()]],
    )
}

fn unpin(keys: &Keys, kind: u16, channel: Uuid, pin: EventId) -> Event {
    event(
        keys,
        kind,
        "",
        &[&["h", &channel.to_string()], &["e", &pin.to_hex()]],
    )
}

fn message(error: IngestError) -> String {
    match error {
        IngestError::Rejected(m) | IngestError::AuthFailed(m) => m,
        other => panic!("unexpected error: {other:?}"),
    }
}

async fn live_pins(state: &AppState, tenant: &TenantContext, channel: Uuid) -> Vec<String> {
    state
        .db
        .query_events(&EventQuery {
            channel_id: Some(channel),
            kinds: Some(vec![KIND_STREAM_MESSAGE_PINNED as i32]),
            ..EventQuery::for_community(tenant.community())
        })
        .await
        .expect("query pins")
        .iter()
        .flat_map(|stored| {
            stored
                .event
                .tags
                .iter()
                .filter(|t| t.kind().to_string() == "e")
                .filter_map(|t| t.content().map(str::to_string))
        })
        .collect()
}

/// Members pin, guests do not, and the pin is readable by `#h`/`#e` query.
/// Mutations: drop the generic write gate for kind 40004 → guest pins → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn members_pin_and_guests_cannot() {
    let state = state().await;
    let tenant = community(&state, "pins-roles").await;
    let (owner, member, guest) = (Keys::generate(), Keys::generate(), Keys::generate());
    let channel = create_channel(&state, &tenant, &owner, &[]).await;
    add(&state, &tenant, &owner, channel, &member, "member").await;
    add(&state, &tenant, &owner, channel, &guest, "guest").await;
    let target = post(&state, &tenant, &owner, channel, 9).await;

    let refused = send(&state, &tenant, &guest, pin(&guest, channel, target))
        .await
        .expect_err("guest pin");
    assert!(message(refused).starts_with("restricted:"));

    send(&state, &tenant, &member, pin(&member, channel, target))
        .await
        .expect("member pin");
    assert_eq!(
        live_pins(&state, &tenant, channel).await,
        vec![target.to_hex()]
    );
}

/// A pin names one live message of its own channel, once.
/// Mutations: skip the channel check → cross-channel pin accepted → RED;
/// skip the kind check → pinning a reaction accepted → RED; skip the
/// duplicate check → second pin accepted → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn pin_must_name_one_live_message_of_the_channel_once() {
    let state = state().await;
    let tenant = community(&state, "pins-target").await;
    let owner = Keys::generate();
    let channel = create_channel(&state, &tenant, &owner, &[]).await;
    let other = create_channel(&state, &tenant, &owner, &[]).await;
    let target = post(&state, &tenant, &owner, channel, 9).await;
    let elsewhere = post(&state, &tenant, &owner, other, 9).await;
    let reaction = event(&owner, 7, "+", &[&["e", &target.to_hex()]]);
    let reaction_id = reaction.id;
    send(&state, &tenant, &owner, reaction)
        .await
        .expect("react");

    let no_e = event(&owner, 40004, "", &[&["h", &channel.to_string()]]);
    let two_e = event(
        &owner,
        40004,
        "",
        &[
            &["h", &channel.to_string()],
            &["e", &target.to_hex()],
            &["e", &elsewhere.to_hex()],
        ],
    );
    let missing = EventId::all_zeros();
    for (label, ev) in [
        ("no e tag", no_e),
        ("two e tags", two_e),
        ("other channel", pin(&owner, channel, elsewhere)),
        ("not a message", pin(&owner, channel, reaction_id)),
        ("missing", pin(&owner, channel, missing)),
    ] {
        let error = send(&state, &tenant, &owner, ev).await.expect_err(label);
        assert!(message(error).starts_with("invalid:"), "{label}");
    }
    assert!(live_pins(&state, &tenant, channel).await.is_empty());

    send(&state, &tenant, &owner, pin(&owner, channel, target))
        .await
        .expect("first pin");
    let again = send(&state, &tenant, &owner, pin(&owner, channel, target))
        .await
        .expect_err("second pin");
    assert_eq!(message(again), "invalid: message is already pinned");
}

/// A pin's author removes it with kind 5 and a channel admin with kind 9005;
/// another member cannot. A removed pin frees the message to be pinned again.
/// Mutations: let any member delete → bystander removes it → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn author_or_admin_unpins_and_the_message_can_be_pinned_again() {
    let state = state().await;
    let tenant = community(&state, "pins-unpin").await;
    let (owner, author, bystander) = (Keys::generate(), Keys::generate(), Keys::generate());
    let channel = create_channel(&state, &tenant, &owner, &[]).await;
    add(&state, &tenant, &owner, channel, &author, "member").await;
    add(&state, &tenant, &owner, channel, &bystander, "member").await;
    let target = post(&state, &tenant, &owner, channel, 40002).await;

    let first = pin(&author, channel, target);
    let first_id = first.id;
    send(&state, &tenant, &author, first).await.expect("pin");

    let denied = send(
        &state,
        &tenant,
        &bystander,
        unpin(&bystander, 9005, channel, first_id),
    )
    .await
    .expect_err("bystander unpin");
    assert!(message(denied).contains("not deletable by you"));
    let denied = send(
        &state,
        &tenant,
        &bystander,
        unpin(&bystander, 5, channel, first_id),
    )
    .await
    .expect_err("bystander kind 5");
    assert!(message(denied).contains("not deletable by you"));
    assert_eq!(live_pins(&state, &tenant, channel).await.len(), 1);

    send(
        &state,
        &tenant,
        &author,
        unpin(&author, 5, channel, first_id),
    )
    .await
    .expect("author unpin");
    assert!(live_pins(&state, &tenant, channel).await.is_empty());

    let second = pin(&bystander, channel, target);
    let second_id = second.id;
    send(&state, &tenant, &bystander, second)
        .await
        .expect("re-pin");
    send(
        &state,
        &tenant,
        &owner,
        unpin(&owner, 9005, channel, second_id),
    )
    .await
    .expect("admin unpin");
    assert!(live_pins(&state, &tenant, channel).await.is_empty());
}

/// A deleted message cannot be pinned.
/// Mutations: read the target with tombstones included → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_deleted_message_cannot_be_pinned() {
    let state = state().await;
    let tenant = community(&state, "pins-deleted").await;
    let owner = Keys::generate();
    let channel = create_channel(&state, &tenant, &owner, &[]).await;
    let target = post(&state, &tenant, &owner, channel, 9).await;
    let delete = event(
        &owner,
        9005,
        "",
        &[&["h", &channel.to_string()], &["e", &target.to_hex()]],
    );
    send(&state, &tenant, &owner, delete).await.expect("delete");

    let error = send(&state, &tenant, &owner, pin(&owner, channel, target))
        .await
        .expect_err("pin deleted");
    assert_eq!(message(error), "invalid: pin target message not found");
}
