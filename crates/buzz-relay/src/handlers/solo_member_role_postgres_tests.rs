//! Signed Solo role commands through the production ingest path.

use buzz_core::{
    kind::{
        KIND_MEMBER_ADDED_NOTIFICATION, KIND_NIP29_GROUP_ADMINS, KIND_NIP29_GROUP_MEMBERS,
        KIND_NIP29_REMOVE_USER, KIND_SOLO_MEMBER_ROLE, KIND_SYSTEM_MESSAGE,
    },
    TenantContext,
};
use buzz_db::{channel::MemberRole, event::EventQuery};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use uuid::Uuid;

use super::{
    group_state_postgres_tests::{community, create_channel, group_state_tags, http_auth, state},
    ingest::{ingest_event, IngestError},
};
use crate::state::AppState;

fn role_change(owner: &Keys, channel_id: Uuid, target: &Keys, expected: &str, next: &str) -> Event {
    EventBuilder::new(Kind::Custom(KIND_SOLO_MEMBER_ROLE as u16), "")
        .tags([
            Tag::parse(["h", &channel_id.to_string()]).unwrap(),
            Tag::parse(["p", &target.public_key().to_hex()]).unwrap(),
            Tag::parse(["expected_role", expected]).unwrap(),
            Tag::parse(["role", next]).unwrap(),
        ])
        .sign_with_keys(owner)
        .expect("sign Solo member role command")
}

async fn record_agent_owner(state: &AppState, tenant: &TenantContext, agent: &Keys, owner: &Keys) {
    let agent = agent.public_key().to_bytes();
    let owner = owner.public_key().to_bytes();
    for pubkey in [&agent, &owner] {
        state
            .db
            .ensure_user(tenant.community(), pubkey)
            .await
            .expect("user");
    }
    assert!(state
        .db
        .set_agent_owner(tenant.community(), &agent, &owner)
        .await
        .expect("owner link"));
}

async fn notice_count(state: &AppState, tenant: &TenantContext, channel_id: Uuid) -> usize {
    state
        .db
        .query_events(&EventQuery {
            kinds: Some(vec![
                KIND_SYSTEM_MESSAGE as i32,
                KIND_MEMBER_ADDED_NOTIFICATION as i32,
            ]),
            custom_tag: Some(("h".into(), channel_id.to_string())),
            ..EventQuery::for_community(tenant.community())
        })
        .await
        .expect("membership notices")
        .len()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn solo_owner_edits_active_members_and_publishes_the_new_roster() {
    let state = state().await;
    let tenant = community(&state, "solo-agent-role").await;
    let [owner, other_owner, own_agent, other_agent, person] =
        std::array::from_fn(|_| Keys::generate());
    record_agent_owner(&state, &tenant, &own_agent, &owner).await;
    record_agent_owner(&state, &tenant, &other_agent, &other_owner).await;
    let channel_id = create_channel(
        &state,
        &tenant,
        &owner,
        &[
            &["visibility", "private"],
            &["t", "buzz.solo"],
            &["p", &own_agent.public_key().to_hex()],
            &["p", &person.public_key().to_hex()],
        ],
    )
    .await;
    for admin in [&other_agent, &person] {
        state
            .db
            .add_member(
                tenant.community(),
                channel_id,
                &admin.public_key().to_bytes(),
                MemberRole::Admin,
                Some(&owner.public_key().to_bytes()),
            )
            .await
            .expect("existing admin");
    }
    state
        .db
        .set_channel_add_policy(
            tenant.community(),
            &other_agent.public_key().to_bytes(),
            "nobody",
        )
        .await
        .expect("change invitation policy");
    let initial_notices = notice_count(&state, &tenant, channel_id).await;

    let downgrade = role_change(&owner, channel_id, &other_agent, "admin", "guest");
    let own_promotion = role_change(&owner, channel_id, &own_agent, "member", "admin");
    let participating = role_change(&owner, channel_id, &other_agent, "guest", "member");
    for (command, target, role) in [
        (downgrade.clone(), &other_agent, "guest"),
        (own_promotion, &own_agent, "admin"),
        (participating, &other_agent, "bot"),
        (
            role_change(&owner, channel_id, &person, "admin", "member"),
            &person,
            "member",
        ),
        (
            role_change(&owner, channel_id, &person, "member", "guest"),
            &person,
            "guest",
        ),
    ] {
        let command_id = command.id;
        let result = ingest_event(&state, &tenant, command, http_auth(&owner))
            .await
            .expect("accepted role change");
        assert!(result.accepted, "{}", result.message);
        assert!(state
            .db
            .get_event_by_id(tenant.community(), command_id.as_bytes())
            .await
            .expect("stored command")
            .is_some());
        let active_role = state
            .db
            .get_member_role(
                tenant.community(),
                channel_id,
                &target.public_key().to_bytes(),
            )
            .await
            .expect("active role");
        assert_eq!(active_role.as_deref(), Some(role));
    }
    let result = ingest_event(&state, &tenant, downgrade, http_auth(&owner))
        .await
        .expect("confirm an already applied command");
    assert_eq!(result.message, "duplicate:");

    for (actor, target, expected, role, reason) in [
        (
            &owner,
            &other_agent,
            "member",
            "guest",
            "membership or role changed",
        ),
        (
            &owner,
            &other_agent,
            "bot",
            "admin",
            "only an agent's owner",
        ),
        (&owner, &person, "guest", "member", "only a verified agent"),
        (
            &own_agent,
            &other_agent,
            "bot",
            "guest",
            "only a Solo chat owner",
        ),
    ] {
        let command = role_change(actor, channel_id, target, expected, role);
        let refused = ingest_event(&state, &tenant, command.clone(), http_auth(actor))
            .await
            .err();
        assert!(
            matches!(refused, Some(IngestError::Rejected(ref message)) if message.contains(reason)),
            "{refused:?}"
        );
        assert!(state
            .db
            .get_event_by_id(tenant.community(), command.id.as_bytes())
            .await
            .expect("query declined command")
            .is_none());
    }

    let roster =
        group_state_tags(&state, &tenant, channel_id, KIND_NIP29_GROUP_MEMBERS as i32).await;
    for (target, role) in [
        (&own_agent, "admin"),
        (&other_agent, "bot"),
        (&person, "guest"),
    ] {
        assert!(roster.contains(&vec![
            "p".into(),
            target.public_key().to_hex(),
            "".into(),
            role.into()
        ]));
    }
    assert_eq!(
        notice_count(&state, &tenant, channel_id).await,
        initial_notices
    );

    let removal = EventBuilder::new(Kind::Custom(KIND_NIP29_REMOVE_USER as u16), "")
        .tags([
            Tag::parse(["h", &channel_id.to_string()]).unwrap(),
            Tag::parse(["p", &other_agent.public_key().to_hex()]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .expect("sign removal");
    let result = ingest_event(&state, &tenant, removal, http_auth(&owner))
        .await
        .expect("remove agent");
    assert!(result.accepted, "{}", result.message);

    let command = role_change(&owner, channel_id, &other_agent, "bot", "guest");
    let command_id = command.id;
    let refused = ingest_event(&state, &tenant, command, http_auth(&owner))
        .await
        .err();
    assert!(
        matches!(refused, Some(IngestError::Rejected(ref message)) if message.contains("membership or role changed")),
        "{refused:?}"
    );
    let roster =
        group_state_tags(&state, &tenant, channel_id, KIND_NIP29_GROUP_MEMBERS as i32).await;
    let removed_key = other_agent.public_key().to_hex();
    assert!(!roster
        .iter()
        .any(|tag| tag[0] == "p" && tag[1] == removed_key));
    assert!(state
        .db
        .get_event_by_id(tenant.community(), command_id.as_bytes())
        .await
        .expect("query declined command")
        .is_none());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn solo_role_change_is_refused_on_an_open_channel() {
    let state = state().await;
    let tenant = community(&state, "solo-role-open-channel").await;
    let [owner, own_agent] = std::array::from_fn(|_| Keys::generate());
    record_agent_owner(&state, &tenant, &own_agent, &owner).await;
    let channel_id = create_channel(
        &state,
        &tenant,
        &owner,
        &[
            &["visibility", "open"],
            &["t", "buzz.solo"],
            &["p", &own_agent.public_key().to_hex()],
        ],
    )
    .await;

    let command = role_change(&owner, channel_id, &own_agent, "member", "guest");
    let command_id = command.id;
    let refused = ingest_event(&state, &tenant, command, http_auth(&owner))
        .await
        .err();
    assert!(
        matches!(refused, Some(IngestError::Rejected(ref message)) if message.contains("channel is not a Solo chat")),
        "{refused:?}"
    );
    assert!(state
        .db
        .get_event_by_id(tenant.community(), command_id.as_bytes())
        .await
        .expect("query declined command")
        .is_none());
    let active_role = state
        .db
        .get_member_role(
            tenant.community(),
            channel_id,
            &own_agent.public_key().to_bytes(),
        )
        .await
        .expect("active role");
    assert_eq!(active_role.as_deref(), Some("member"));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn solo_role_change_rolls_back_if_signed_roster_write_fails() {
    let state = state().await;
    let tenant = community(&state, "solo-role-roster-failure").await;
    let [owner, agent] = std::array::from_fn(|_| Keys::generate());
    record_agent_owner(&state, &tenant, &agent, &owner).await;
    let channel_id = create_channel(
        &state,
        &tenant,
        &owner,
        &[
            &["visibility", "private"],
            &["t", "buzz.solo"],
            &["p", &agent.public_key().to_hex()],
        ],
    )
    .await;
    let admins_kind = KIND_NIP29_GROUP_ADMINS as i32;
    let members_kind = KIND_NIP29_GROUP_MEMBERS as i32;
    let before_admins = group_state_tags(&state, &tenant, channel_id, admins_kind).await;
    let before_members = group_state_tags(&state, &tenant, channel_id, members_kind).await;

    // The PostgreSQL runner isolates this trigger to the test's database.
    for statement in [
        "CREATE FUNCTION solo_roster_fail() RETURNS trigger LANGUAGE plpgsql AS \
         $$ BEGIN RAISE EXCEPTION 'injected roster write failure'; END $$",
        "CREATE TRIGGER solo_roster_fail BEFORE INSERT ON events FOR EACH ROW \
         WHEN (NEW.kind = 39002) EXECUTE FUNCTION solo_roster_fail()",
    ] {
        sqlx::query(statement)
            .execute(state.db.pool())
            .await
            .expect("install roster failure trigger");
    }

    let command = role_change(&owner, channel_id, &agent, "member", "admin");
    let command_id = command.id;
    let error = ingest_event(&state, &tenant, command.clone(), http_auth(&owner))
        .await
        .err();
    assert!(
        matches!(error, Some(IngestError::Internal(ref message)) if message.contains("injected roster write failure")),
        "{error:?}"
    );
    assert!(state
        .db
        .get_event_by_id(tenant.community(), command_id.as_bytes())
        .await
        .expect("query failed command")
        .is_none());
    let active_role = state
        .db
        .get_member_role(
            tenant.community(),
            channel_id,
            &agent.public_key().to_bytes(),
        )
        .await
        .expect("active role after rollback");
    assert_eq!(active_role.as_deref(), Some("member"));
    assert_eq!(
        group_state_tags(&state, &tenant, channel_id, admins_kind).await,
        before_admins
    );
    assert_eq!(
        group_state_tags(&state, &tenant, channel_id, members_kind).await,
        before_members
    );

    sqlx::query("DROP TRIGGER solo_roster_fail ON events")
        .execute(state.db.pool())
        .await
        .expect("remove roster failure trigger");
    let retry = ingest_event(&state, &tenant, command, http_auth(&owner))
        .await
        .expect("retry the same signed command");
    assert!(
        retry.accepted && retry.message != "duplicate:",
        "{}",
        retry.message
    );
    let agent_hex = agent.public_key().to_hex();
    let admins = group_state_tags(&state, &tenant, channel_id, admins_kind).await;
    assert!(admins.contains(&vec!["p".into(), agent_hex.clone(), "admin".into()]));
    let members = group_state_tags(&state, &tenant, channel_id, members_kind).await;
    assert!(members.contains(&vec!["p".into(), agent_hex, "".into(), "admin".into()]));
}
