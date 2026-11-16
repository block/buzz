//! Postgres-backed tests for starting a channel with its members: a kind:9007
//! that adds its `p` tags as members, and a creator's retry of it.

use std::sync::Arc;

use buzz_core::TenantContext;
use buzz_db::event::EventQuery;
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use uuid::Uuid;

use crate::handlers::group_state_postgres_tests::{community, group_state_tags, http_auth, state};
use crate::handlers::ingest::{ingest_event, IngestError, IngestResult};
use crate::state::AppState;

fn create_event(creator: &Keys, channel_id: Uuid, members: &[&Keys]) -> Event {
    let mut tags = vec![
        Tag::parse(["h", &channel_id.to_string()]).unwrap(),
        Tag::parse(["name", "start"]).unwrap(),
        Tag::parse(["visibility", "private"]).unwrap(),
    ];
    for member in members {
        tags.push(Tag::parse(["p", &member.public_key().to_hex()]).unwrap());
    }
    EventBuilder::new(Kind::Custom(9007), "")
        .tags(tags)
        .sign_with_keys(creator)
        .expect("sign 9007")
}

fn message_event(author: &Keys, channel_id: Uuid) -> Event {
    EventBuilder::new(Kind::Custom(9), "hello")
        .tags([Tag::parse(["h", &channel_id.to_string()]).unwrap()])
        .sign_with_keys(author)
        .expect("sign message")
}

async fn ingest(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    event: Event,
    keys: &Keys,
) -> Result<IngestResult, IngestError> {
    let mut auth = http_auth(keys);
    if let crate::handlers::ingest::IngestAuth::Http { scopes, .. } = &mut auth {
        scopes.push(buzz_auth::Scope::MessagesRead);
    }
    ingest_event(state, tenant, event, auth).await
}

async fn roles(
    state: &AppState,
    tenant: &TenantContext,
    channel_id: Uuid,
) -> Vec<(Vec<u8>, String)> {
    let mut roles: Vec<_> = state
        .db
        .get_members(tenant.community(), channel_id)
        .await
        .expect("members")
        .into_iter()
        .map(|member| (member.pubkey, member.role))
        .collect();
    roles.sort();
    roles
}

/// The `p` tags join in the create's transaction, the roster lists them, and
/// each gets the "joined" notice a kind:9000 would post.
/// Mutation: drop `members` from the create → no agent membership → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn create_adds_its_p_tags_as_members() {
    let state = state().await;
    let tenant = community(&state, "create-members").await;
    let (creator, agent) = (Keys::generate(), Keys::generate());
    let channel_id = Uuid::new_v4();

    let result = ingest(
        &state,
        &tenant,
        create_event(&creator, channel_id, &[&agent, &agent]),
        &creator,
    )
    .await
    .expect("create");
    assert!(result.accepted, "{}", result.message);

    let mut expected = vec![
        (
            creator.public_key().to_bytes().to_vec(),
            "owner".to_string(),
        ),
        (agent.public_key().to_bytes().to_vec(), "member".to_string()),
    ];
    expected.sort();
    assert_eq!(roles(&state, &tenant, channel_id).await, expected);

    let roster = group_state_tags(&state, &tenant, channel_id, 39002).await;
    assert!(
        roster
            .iter()
            .any(|tag| tag[0] == "p" && tag[1] == agent.public_key().to_hex()),
        "{roster:?}"
    );

    let notices = state
        .db
        .query_events(&EventQuery {
            kinds: Some(vec![buzz_core::kind::KIND_SYSTEM_MESSAGE as i32]),
            channel_id: Some(channel_id),
            ..EventQuery::for_community(tenant.community())
        })
        .await
        .expect("system messages");
    let joined = notices
        .iter()
        .filter(|stored| {
            let content: serde_json::Value =
                serde_json::from_str(&stored.event.content).unwrap_or_default();
            content["type"] == "member_joined" && content["target"] == agent.public_key().to_hex()
        })
        .count();
    assert_eq!(joined, 1, "one joined notice for the agent");

    // The agent can answer in the chat with no separate add.
    let reply = ingest(&state, &tenant, message_event(&agent, channel_id), &agent)
        .await
        .expect("agent reply");
    assert!(reply.accepted, "{}", reply.message);
}

/// An agent whose policy refuses third-party adds keeps the whole create out:
/// no channel is made. Mutation: skip the policy check → channel exists → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn create_respects_the_members_add_policy() {
    let state = state().await;
    let tenant = community(&state, "create-members-policy").await;
    let (creator, agent) = (Keys::generate(), Keys::generate());
    let agent_bytes = agent.public_key().to_bytes().to_vec();
    state
        .db
        .ensure_user(tenant.community(), &agent_bytes)
        .await
        .expect("agent user");
    state
        .db
        .set_channel_add_policy(tenant.community(), &agent_bytes, "nobody")
        .await
        .expect("policy");
    let channel_id = Uuid::new_v4();

    let result = ingest(
        &state,
        &tenant,
        create_event(&creator, channel_id, &[&agent]),
        &creator,
    )
    .await;
    match result {
        Err(IngestError::Rejected(message)) => {
            assert!(message.starts_with("invalid:"), "{message}")
        }
        Err(error) => panic!("expected a rejection, got {error:?}"),
        Ok(result) => panic!("expected a rejection, got OK {}", result.message),
    }
    assert!(state
        .db
        .get_channel(tenant.community(), channel_id)
        .await
        .is_err());
}

/// Resending a create whose OK was lost is accepted; a different create for
/// the same UUID, even from its creator, is refused before its side effects,
/// so it can neither add nor announce anyone.
/// Mutations: refuse every existing UUID → resend not accepted → RED; check the
/// members' policies before recognizing a stored resend → RED; let the
/// creator's changed create through → stranger announced → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn resending_a_create_confirms_it() {
    let state = state().await;
    let tenant = community(&state, "create-retry").await;
    let (creator, agent, stranger, other) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    let channel_id = Uuid::new_v4();
    let create = create_event(&creator, channel_id, &[&agent]);

    let first = ingest(&state, &tenant, create.clone(), &creator)
        .await
        .expect("create");
    assert!(first.accepted, "{}", first.message);
    let before = roles(&state, &tenant, channel_id).await;
    let resent = ingest(&state, &tenant, create.clone(), &creator)
        .await
        .expect("resend");
    assert!(resent.accepted, "{}", resent.message);

    for (author, members) in [(&creator, vec![&agent, &stranger]), (&other, vec![])] {
        let changed = ingest(
            &state,
            &tenant,
            create_event(author, channel_id, &members),
            author,
        )
        .await
        .expect("changed create");
        assert!(!changed.accepted);
        assert_eq!(changed.message, "duplicate: channel already exists");
    }

    // The agent closing its adds after the create doesn't refuse the resend.
    let agent_bytes = agent.public_key().to_bytes().to_vec();
    state
        .db
        .ensure_user(tenant.community(), &agent_bytes)
        .await
        .expect("agent user");
    state
        .db
        .set_channel_add_policy(tenant.community(), &agent_bytes, "nobody")
        .await
        .expect("policy");
    let resent_after_policy = ingest(&state, &tenant, create, &creator)
        .await
        .expect("resend after policy change");
    assert!(
        resent_after_policy.accepted,
        "{}",
        resent_after_policy.message
    );

    assert_eq!(roles(&state, &tenant, channel_id).await, before);
    let notices = state
        .db
        .query_events(&EventQuery {
            kinds: Some(vec![buzz_core::kind::KIND_SYSTEM_MESSAGE as i32]),
            channel_id: Some(channel_id),
            ..EventQuery::for_community(tenant.community())
        })
        .await
        .expect("system messages");
    assert!(
        !notices.iter().any(|stored| stored
            .event
            .content
            .contains(&stranger.public_key().to_hex())),
        "the refused create announced nobody"
    );
}

/// Creation persists live ancestry and retries repair signed metadata without
/// copying source members. An inaccessible source or wrong cutoff writes nothing.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn solo_fork_preserves_authorized_pointer_and_confirms_exact_retry() {
    let state = state().await;
    let tenant = community(&state, "solo-fork").await;
    let (creator, outsider, agent) = (Keys::generate(), Keys::generate(), Keys::generate());
    let source = Uuid::new_v4();
    assert!(
        ingest(
            &state,
            &tenant,
            create_event(&creator, source, &[&agent]),
            &creator
        )
        .await
        .unwrap()
        .accepted
    );
    let cutoff = message_event(&creator, source);
    assert!(
        ingest(&state, &tenant, cutoff.clone(), &creator)
            .await
            .unwrap()
            .accepted
    );
    let fork_create = |author: &Keys, destination: Uuid, message_id: String| {
        EventBuilder::new(Kind::Custom(9007), "")
            .tags([
                Tag::parse(["h", &destination.to_string()]).unwrap(),
                Tag::parse(["name", "fork"]).unwrap(),
                Tag::parse(["visibility", "private"]).unwrap(),
                Tag::parse(["channel_type", "stream"]).unwrap(),
                Tag::parse(["forked_from", &source.to_string(), &message_id]).unwrap(),
                Tag::parse(["t", "buzz.solo"]).unwrap(),
            ])
            .sign_with_keys(author)
            .unwrap()
    };
    for (author, message_id) in [(&outsider, cutoff.id.to_hex()), (&creator, "ff".repeat(32))] {
        let destination = Uuid::new_v4();
        assert!(ingest(
            &state,
            &tenant,
            fork_create(author, destination, message_id),
            author
        )
        .await
        .is_err());
        assert!(state
            .db
            .get_channel(tenant.community(), destination)
            .await
            .is_err());
    }
    let destination = Uuid::new_v4();
    let create = fork_create(&creator, destination, cutoff.id.to_hex());
    // Fail the real signed-metadata insert, after the atomic create commits.
    // This database belongs to this test, so the failure cannot affect another journey.
    sqlx::query(
        "ALTER TABLE events ADD CONSTRAINT solo_fork_metadata_failure \
         CHECK (kind <> 39000) NOT VALID",
    )
    .execute(state.db.pool())
    .await
    .unwrap();
    assert!(matches!(
        ingest(&state, &tenant, create.clone(), &creator).await,
        Err(IngestError::Internal(message)) if message.contains("fork metadata confirmation failed")
    ));
    let record = state
        .db
        .get_channel(tenant.community(), destination)
        .await
        .unwrap();
    assert_eq!(record.fork_source_channel_id, Some(source));
    assert_eq!(
        record.fork_source_message_id,
        Some(cutoff.id.as_bytes().to_vec())
    );
    assert_eq!(roles(&state, &tenant, destination).await.len(), 1);
    let expected = vec![
        "forked_from".to_owned(),
        source.to_string(),
        cutoff.id.to_hex(),
    ];
    sqlx::query("ALTER TABLE events DROP CONSTRAINT solo_fork_metadata_failure")
        .execute(state.db.pool())
        .await
        .unwrap();
    assert!(
        ingest(&state, &tenant, create.clone(), &creator)
            .await
            .unwrap()
            .accepted
    );
    assert!(group_state_tags(&state, &tenant, destination, 39000)
        .await
        .contains(&expected));
    let notices_before = state
        .db
        .query_events(&EventQuery {
            kinds: Some(vec![buzz_core::kind::KIND_SYSTEM_MESSAGE as i32]),
            channel_id: Some(destination),
            ..EventQuery::for_community(tenant.community())
        })
        .await
        .unwrap()
        .len();

    // A missing derived metadata event is repaired by the exact create retry.
    sqlx::query("UPDATE events SET deleted_at = NOW() WHERE community_id = $1 AND channel_id = $2 AND kind = 39000")
        .bind(tenant.community().as_uuid()).bind(destination)
        .execute(state.db.pool()).await.unwrap();
    assert!(
        ingest(&state, &tenant, create, &creator)
            .await
            .unwrap()
            .accepted
    );
    assert!(group_state_tags(&state, &tenant, destination, 39000)
        .await
        .contains(&expected));
    assert_eq!(
        state
            .db
            .query_events(&EventQuery {
                kinds: Some(vec![buzz_core::kind::KIND_SYSTEM_MESSAGE as i32]),
                channel_id: Some(destination),
                ..EventQuery::for_community(tenant.community())
            })
            .await
            .unwrap()
            .len(),
        notices_before
    );
    let edit = EventBuilder::new(Kind::Custom(9002), "")
        .tags([
            Tag::parse(["h", &destination.to_string()]).unwrap(),
            Tag::parse(["name", "renamed fork"]).unwrap(),
            Tag::parse(["forked_from", &source.to_string(), &"ff".repeat(32)]).unwrap(),
        ])
        .sign_with_keys(&creator)
        .unwrap();
    assert!(ingest(&state, &tenant, edit, &creator).await.is_err());
    assert_eq!(
        state
            .db
            .get_channel(tenant.community(), destination)
            .await
            .unwrap()
            .fork_source_message_id,
        Some(cutoff.id.as_bytes().to_vec())
    );
    let other_tenant = community(&state, "solo-fork-other").await;
    let cross_destination = Uuid::new_v4();
    assert!(ingest(
        &state,
        &other_tenant,
        fork_create(&creator, cross_destination, cutoff.id.to_hex()),
        &creator
    )
    .await
    .is_err());
    assert!(state
        .db
        .get_channel(other_tenant.community(), cross_destination)
        .await
        .is_err());
    let changed = fork_create(&creator, destination, "ff".repeat(32));
    assert!(
        !ingest(&state, &tenant, changed, &creator)
            .await
            .unwrap()
            .accepted
    );
}
