//! Postgres-backed tests for the `member_joined` and `member_left` kind:40099
//! notices of an announce channel (open, `join_role = guest`): a person who
//! joins as a guest, and a guest who leaves, post none. Everything else about
//! a join or a leave still happens: the roster (kind:39002), the membership
//! notification (kind:44100 or 44101) and the membership cache.

use std::sync::Arc;

use buzz_core::kind::{
    KIND_MEMBER_ADDED_NOTIFICATION, KIND_MEMBER_REMOVED_NOTIFICATION, KIND_SYSTEM_MESSAGE,
};
use buzz_core::TenantContext;
use buzz_db::channel::ChannelUpdate;
use buzz_db::event::EventQuery;
use nostr::{EventBuilder, Keys, Kind, Tag};
use uuid::Uuid;

use crate::handlers::group_state_postgres_tests::{
    community, create_channel, edit_channel, group_state_tags, http_auth, state,
};
use crate::handlers::ingest::ingest_event;
use crate::handlers::side_effects::{auto_join_default_channels, auto_join_never_joined_channels};
use crate::state::AppState;

/// An open channel whose `join_role` is `guest`, made by `owner`.
async fn announce_channel(state: &Arc<AppState>, tenant: &TenantContext, owner: &Keys) -> Uuid {
    let channel_id = create_channel(state, tenant, owner, &[]).await;
    edit_channel(state, tenant, owner, channel_id, &[&["join_role", "guest"]])
        .await
        .expect("make the channel an announce channel");
    channel_id
}

/// Ingest a kind that `keys` signs, with `tags`.
async fn publish(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    keys: &Keys,
    kind: u16,
    tags: &[Vec<String>],
) {
    let tags: Vec<Tag> = tags
        .iter()
        .map(|tag| Tag::parse(tag.iter().map(String::as_str)).unwrap())
        .collect();
    let event = EventBuilder::new(Kind::Custom(kind), "")
        .tags(tags)
        .allow_self_tagging()
        .sign_with_keys(keys)
        .expect("sign");
    let result = ingest_event(state, tenant, event, http_auth(keys))
        .await
        .unwrap_or_else(|error| panic!("ingest kind:{kind}: {error:?}"));
    assert!(result.accepted, "kind:{kind} refused: {}", result.message);
}

fn h(channel_id: Uuid) -> Vec<String> {
    vec!["h".into(), channel_id.to_string()]
}

async fn join(state: &Arc<AppState>, tenant: &TenantContext, keys: &Keys, channel_id: Uuid) {
    publish(state, tenant, keys, 9021, &[h(channel_id)]).await;
}

async fn leave(state: &Arc<AppState>, tenant: &TenantContext, keys: &Keys, channel_id: Uuid) {
    publish(state, tenant, keys, 9022, &[h(channel_id)]).await;
}

/// A kind:9001 `actor` signs to remove `target` (themselves, for a leave).
async fn remove(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    actor: &Keys,
    target: &Keys,
    channel_id: Uuid,
) {
    let p = vec!["p".into(), target.public_key().to_hex()];
    publish(state, tenant, actor, 9001, &[h(channel_id), p]).await;
}

/// A kind:9000 `actor` signs naming `people`, each with an optional role.
async fn add(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    actor: &Keys,
    channel_id: Uuid,
    people: &[(&Keys, Option<&str>)],
) {
    let mut tags = vec![h(channel_id)];
    for (person, role) in people {
        let mut tag = vec!["p".to_owned(), person.public_key().to_hex()];
        tag.extend(role.map(str::to_owned));
        tags.push(tag);
    }
    publish(state, tenant, actor, 9000, &tags).await;
}

async fn role_of(
    state: &AppState,
    tenant: &TenantContext,
    channel_id: Uuid,
    keys: &Keys,
) -> Option<String> {
    state
        .db
        .get_member_role(
            tenant.community(),
            channel_id,
            &keys.public_key().to_bytes(),
        )
        .await
        .expect("member role")
}

/// The kind:40099 notices of `notice_type` in `channel_id` about `who`: the
/// target of a `member_joined`, the actor of a `member_left`.
async fn notices(
    state: &AppState,
    tenant: &TenantContext,
    channel_id: Uuid,
    notice_type: &str,
    who: &Keys,
) -> usize {
    let field = if notice_type == "member_left" {
        "actor"
    } else {
        "target"
    };
    state
        .db
        .query_events(&EventQuery {
            kinds: Some(vec![KIND_SYSTEM_MESSAGE as i32]),
            channel_id: Some(channel_id),
            ..EventQuery::for_community(tenant.community())
        })
        .await
        .expect("system messages")
        .iter()
        .filter(|stored| {
            let content: serde_json::Value =
                serde_json::from_str(&stored.event.content).unwrap_or_default();
            content["type"] == notice_type && content[field] == who.public_key().to_hex()
        })
        .count()
}

async fn joined(state: &AppState, tenant: &TenantContext, channel_id: Uuid, who: &Keys) -> usize {
    notices(state, tenant, channel_id, "member_joined", who).await
}

/// The relay-signed kind:44100 / 44101 `kind` notices telling `who` about
/// `channel_id`.
async fn membership_notifications(
    state: &AppState,
    tenant: &TenantContext,
    kind: u32,
    channel_id: Uuid,
    who: &Keys,
) -> usize {
    state
        .db
        .query_events(&EventQuery {
            kinds: Some(vec![kind as i32]),
            ..EventQuery::for_community(tenant.community())
        })
        .await
        .expect("membership notifications")
        .iter()
        .filter(|stored| {
            let tag = |name: &str, value: &str| {
                stored.event.tags.iter().any(|t| {
                    t.as_slice().first().map(String::as_str) == Some(name)
                        && t.content() == Some(value)
                })
            };
            tag("h", &channel_id.to_string()) && tag("p", &who.public_key().to_hex())
        })
        .count()
}

/// Whether the channel's kind:39002 roster lists `who`.
async fn on_roster(state: &AppState, tenant: &TenantContext, channel_id: Uuid, who: &Keys) -> bool {
    let hex = who.public_key().to_hex();
    group_state_tags(state, tenant, channel_id, 39002)
        .await
        .iter()
        .any(|tag| tag[0] == "p" && tag[1] == hex)
}

/// A kind:9021 into an announce channel makes the joiner a guest and posts no
/// `member_joined`; the roster, the joiner's kind:44100 and the membership
/// cache still update.
/// Mutation: make `join_posts_notice` always true → a notice (RED).
#[tokio::test]
#[ignore = "requires Postgres"]
async fn guest_self_join_in_announce_channel_posts_no_notice() {
    let state = state().await;
    let tenant = community(&state, "quiet-self-join").await;
    let (owner, joiner) = (Keys::generate(), Keys::generate());
    let channel_id = announce_channel(&state, &tenant, &owner).await;
    let joiner_bytes = joiner.public_key().to_bytes();
    assert!(
        !state
            .is_member_cached(tenant.community(), channel_id, &joiner_bytes)
            .await
            .expect("membership"),
        "not a member before the join"
    );

    join(&state, &tenant, &joiner, channel_id).await;

    assert_eq!(
        role_of(&state, &tenant, channel_id, &joiner)
            .await
            .as_deref(),
        Some("guest")
    );
    assert_eq!(joined(&state, &tenant, channel_id, &joiner).await, 0);
    assert!(on_roster(&state, &tenant, channel_id, &joiner).await);
    assert_eq!(
        membership_notifications(
            &state,
            &tenant,
            KIND_MEMBER_ADDED_NOTIFICATION,
            channel_id,
            &joiner
        )
        .await,
        1
    );
    assert!(
        state
            .is_member_cached(tenant.community(), channel_id, &joiner_bytes)
            .await
            .expect("membership"),
        "the cache sees the new member"
    );
}

/// A kind:9000 that makes someone a guest in an announce channel, with no
/// role tag or as a self-add, posts no notice, alone or in a batch with a
/// person made a member, who still gets theirs. Each gets a kind:44100.
/// Mutation: ignore `announce` in the kind:9000 path (post always) → a
/// notice for the guests (RED).
#[tokio::test]
#[ignore = "requires Postgres"]
async fn guest_adds_in_announce_channel_post_no_notice() {
    let state = state().await;
    let tenant = community(&state, "quiet-add").await;
    let (owner, guest, selfer, colleague) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    let channel_id = announce_channel(&state, &tenant, &owner).await;

    // No role tag: the channel's `join_role` decides, with a member's add and
    // with a self-add alike.
    add(&state, &tenant, &owner, channel_id, &[(&guest, None)]).await;
    add(&state, &tenant, &selfer, channel_id, &[(&selfer, None)]).await;
    // One event: a guest by default and a member by role.
    add(
        &state,
        &tenant,
        &owner,
        channel_id,
        &[(&Keys::generate(), None), (&colleague, Some("member"))],
    )
    .await;

    for quiet in [&guest, &selfer] {
        assert_eq!(
            role_of(&state, &tenant, channel_id, quiet).await.as_deref(),
            Some("guest")
        );
        assert_eq!(joined(&state, &tenant, channel_id, quiet).await, 0);
        assert!(on_roster(&state, &tenant, channel_id, quiet).await);
        assert_eq!(
            membership_notifications(
                &state,
                &tenant,
                KIND_MEMBER_ADDED_NOTIFICATION,
                channel_id,
                quiet
            )
            .await,
            1
        );
    }
    assert_eq!(
        role_of(&state, &tenant, channel_id, &colleague)
            .await
            .as_deref(),
        Some("member")
    );
    assert_eq!(
        joined(&state, &tenant, channel_id, &colleague).await,
        1,
        "a person added as a member is announced"
    );
}

/// Joins that are not a guest's join to an announce channel still post: a
/// self-join and an explicit guest add in a normal open channel, a guest add
/// in a private channel whose `join_role` is guest, a promotion to member,
/// and a role change (here a demotion) of someone already in the channel.
/// Mutations: treat every channel as announce in the kind:9000 path → the
/// explicit guest add is quiet (RED); drop the visibility test from
/// `ChannelRecord::is_announce` → the private add is quiet (RED); drop the
/// `prior.is_some()` test → the demotion is quiet (RED).
#[tokio::test]
#[ignore = "requires Postgres"]
async fn joins_that_are_not_announce_guest_joins_still_post() {
    let state = state().await;
    let tenant = community(&state, "quiet-still-posts").await;
    let owner = Keys::generate();
    let (joiner, explicit_guest, private_guest) =
        (Keys::generate(), Keys::generate(), Keys::generate());

    let normal = create_channel(&state, &tenant, &owner, &[]).await;
    join(&state, &tenant, &joiner, normal).await;
    assert_eq!(
        role_of(&state, &tenant, normal, &joiner).await.as_deref(),
        Some("member")
    );
    assert_eq!(joined(&state, &tenant, normal, &joiner).await, 1);
    add(
        &state,
        &tenant,
        &owner,
        normal,
        &[(&explicit_guest, Some("guest"))],
    )
    .await;
    assert_eq!(
        role_of(&state, &tenant, normal, &explicit_guest)
            .await
            .as_deref(),
        Some("guest")
    );
    assert_eq!(joined(&state, &tenant, normal, &explicit_guest).await, 1);

    let private = create_channel(&state, &tenant, &owner, &[&["visibility", "private"]]).await;
    edit_channel(&state, &tenant, &owner, private, &[&["join_role", "guest"]])
        .await
        .expect("private channel with join_role guest");
    add(&state, &tenant, &owner, private, &[(&private_guest, None)]).await;
    assert_eq!(
        role_of(&state, &tenant, private, &private_guest)
            .await
            .as_deref(),
        Some("guest")
    );
    assert_eq!(joined(&state, &tenant, private, &private_guest).await, 1);

    // A guest joined quietly is announced when a member makes them a member,
    // and a role change after that posts again.
    let announce = announce_channel(&state, &tenant, &owner).await;
    let guest = Keys::generate();
    join(&state, &tenant, &guest, announce).await;
    assert_eq!(joined(&state, &tenant, announce, &guest).await, 0);
    add(
        &state,
        &tenant,
        &owner,
        announce,
        &[(&guest, Some("member"))],
    )
    .await;
    assert_eq!(
        role_of(&state, &tenant, announce, &guest).await.as_deref(),
        Some("member")
    );
    assert_eq!(joined(&state, &tenant, announce, &guest).await, 1);
    // A notice is keyed by its content and second, so a second one about the
    // same person and actor needs a later second to be stored at all.
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    add(
        &state,
        &tenant,
        &owner,
        announce,
        &[(&guest, Some("guest"))],
    )
    .await;
    assert_eq!(
        role_of(&state, &tenant, announce, &guest).await.as_deref(),
        Some("guest")
    );
    assert_eq!(joined(&state, &tenant, announce, &guest).await, 2);
}

/// A guest who leaves an announce channel, by kind:9022 or by removing
/// themselves with kind:9001, posts no `member_left`; the roster and the
/// leaver's kind:44101 still update. Every other departure still posts: a
/// member leaving the announce channel, a member and a guest leaving a normal
/// channel, and a kick of a guest.
/// Mutation: make `is_guest_leaving_announce_channel` always false → a
/// `member_left` for the guests (RED); always true → none for the members
/// (RED).
#[tokio::test]
#[ignore = "requires Postgres"]
async fn guest_leaving_an_announce_channel_posts_no_notice() {
    let state = state().await;
    let tenant = community(&state, "quiet-leave").await;
    let owner = Keys::generate();
    let announce = announce_channel(&state, &tenant, &owner).await;
    let normal = create_channel(&state, &tenant, &owner, &[]).await;
    let (guest_9022, guest_9001, kicked, member, normal_member, normal_guest) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    for guest in [&guest_9022, &guest_9001, &kicked] {
        join(&state, &tenant, guest, announce).await;
    }
    add(
        &state,
        &tenant,
        &owner,
        announce,
        &[(&member, Some("member"))],
    )
    .await;
    join(&state, &tenant, &normal_member, normal).await;
    add(
        &state,
        &tenant,
        &owner,
        normal,
        &[(&normal_guest, Some("guest"))],
    )
    .await;

    leave(&state, &tenant, &guest_9022, announce).await;
    remove(&state, &tenant, &guest_9001, &guest_9001, announce).await;
    leave(&state, &tenant, &member, announce).await;
    leave(&state, &tenant, &normal_member, normal).await;
    leave(&state, &tenant, &normal_guest, normal).await;
    remove(&state, &tenant, &owner, &kicked, announce).await;

    for gone in [&guest_9022, &guest_9001] {
        assert_eq!(role_of(&state, &tenant, announce, gone).await, None);
        assert_eq!(
            notices(&state, &tenant, announce, "member_left", gone).await,
            0
        );
        assert!(!on_roster(&state, &tenant, announce, gone).await);
        assert_eq!(
            membership_notifications(
                &state,
                &tenant,
                KIND_MEMBER_REMOVED_NOTIFICATION,
                announce,
                gone
            )
            .await,
            1
        );
    }
    for (channel_id, leaver) in [
        (announce, &member),
        (normal, &normal_member),
        (normal, &normal_guest),
    ] {
        assert_eq!(
            notices(&state, &tenant, channel_id, "member_left", leaver).await,
            1
        );
    }
    assert_eq!(
        notices(&state, &tenant, announce, "member_removed", &kicked).await,
        1,
        "a kick of a guest is announced"
    );
}

/// The relay's default-channel joins into an announce channel post no
/// `member_joined`, by either path: a new community member's joins and an
/// admission's join of someone who was never a member. A default channel that
/// is not an announce channel still announces both, and each person still
/// gets a roster entry and a kind:44100 for the announce channel.
/// Mutation: make `join_posts_notice` always true → a notice in the announce
/// channel (RED).
#[tokio::test]
#[ignore = "requires Postgres"]
async fn auto_join_into_an_announce_default_channel_posts_no_notice() {
    let state = state().await;
    let tenant = community(&state, "quiet-auto-join").await;
    let owner = Keys::generate();
    let announce = announce_channel(&state, &tenant, &owner).await;
    let normal = create_channel(&state, &tenant, &owner, &[]).await;
    for channel_id in [announce, normal] {
        state
            .db
            .update_channel(
                tenant.community(),
                channel_id,
                ChannelUpdate {
                    auto_join: Some(true),
                    ..Default::default()
                },
            )
            .await
            .expect("flag channel as a default");
    }

    let (member, admitted) = (Keys::generate(), Keys::generate());
    auto_join_default_channels(&tenant, &state, &member.public_key().to_hex(), None).await;
    let gate = crate::nip_fi_gate::SessionAdmissionGate::off_mode(
        tokio_util::sync::CancellationToken::new(),
    );
    auto_join_never_joined_channels(&tenant, &state, &admitted.public_key().to_bytes(), &gate)
        .await;

    for person in [&member, &admitted] {
        assert_eq!(
            role_of(&state, &tenant, announce, person).await.as_deref(),
            Some("guest")
        );
        assert_eq!(joined(&state, &tenant, announce, person).await, 0);
        assert!(on_roster(&state, &tenant, announce, person).await);
        assert_eq!(
            membership_notifications(
                &state,
                &tenant,
                KIND_MEMBER_ADDED_NOTIFICATION,
                announce,
                person
            )
            .await,
            1
        );
        assert_eq!(
            role_of(&state, &tenant, normal, person).await.as_deref(),
            Some("member")
        );
        assert_eq!(joined(&state, &tenant, normal, person).await, 1);
    }
}
