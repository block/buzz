//! Postgres-backed tests for starting a channel with its members: a kind:9007
//! that adds its `p` tags as members, and a creator's retry of it.

use std::sync::Arc;

use buzz_core::TenantContext;
use buzz_db::event::EventQuery;
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use uuid::Uuid;

use crate::handlers::group_state_postgres_tests::{
    community, group_state_tags, http_auth, req_event_ids, state,
};
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
    ingest_event(state, tenant, event, http_auth(keys)).await
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

/// Generic private DMs remain supported, with or without an explicit
/// visibility. An explicit open DM is refused without durable side effects.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn dm_visibility_create_requests_are_atomic() {
    let state = state().await;
    let tenant = community(&state, "dm-visibility-create").await;
    let (creator, participant, outsider) = (Keys::generate(), Keys::generate(), Keys::generate());
    for visibility in [Some("open"), None, Some("private")] {
        let channel_id = Uuid::new_v4();
        let mut tags = vec![
            Tag::parse(["h", &channel_id.to_string()]).unwrap(),
            Tag::parse(["name", "Direct message"]).unwrap(),
            Tag::parse(["channel_type", "dm"]).unwrap(),
            Tag::parse(["p", &participant.public_key().to_hex()]).unwrap(),
        ];
        if let Some(visibility) = visibility {
            tags.push(Tag::parse(["visibility", visibility]).unwrap());
        }
        let event = EventBuilder::new(Kind::Custom(9007), "")
            .tags(tags)
            .sign_with_keys(&creator)
            .expect("sign DM creation");
        let result = ingest(&state, &tenant, event, &creator).await;
        if visibility == Some("open") {
            let error = result.err().expect("open DM must be refused");
            assert!(
                matches!(error, IngestError::Rejected(ref reason)
                if reason == "invalid: DMs must be private"),
                "{error:?}"
            );
            let counts: (i64, i64, i64) = sqlx::query_as(
                "SELECT (SELECT count(*) FROM channels WHERE community_id=$1), \
                        (SELECT count(*) FROM channel_members WHERE community_id=$1), \
                        (SELECT count(*) FROM events WHERE community_id=$1)",
            )
            .bind(tenant.community().as_uuid())
            .fetch_one(state.db.pool())
            .await
            .expect("count refused create effects");
            assert_eq!(
                counts,
                (0, 0, 0),
                "no channel, roster, event or notice survives"
            );
            continue;
        }
        assert!(result.expect("private DM creation").accepted);
        let channel = state
            .db
            .get_channel(tenant.community(), channel_id)
            .await
            .expect("DM");
        assert_eq!(channel.channel_type, "dm");
        assert_eq!(channel.visibility, "private");
        assert_eq!(roles(&state, &tenant, channel_id).await.len(), 2);
        let metadata = group_state_tags(&state, &tenant, channel_id, 39000).await;
        assert!(metadata.iter().any(|tag| tag[0] == "private"));
        assert!(!metadata.iter().any(|tag| tag[0] == "public"));

        let message = message_event(&participant, channel_id);
        let id = message.id.to_hex();
        assert!(
            ingest(&state, &tenant, message, &participant)
                .await
                .expect("participant posts")
                .accepted
        );
        for reader in [&creator, &participant] {
            let ids = req_event_ids(
                &state,
                &tenant,
                reader,
                serde_json::json!({"kinds": [9], "#h": [channel_id.to_string()]}),
            )
            .await;
            assert!(ids.contains(&id), "participants can read their DM");
        }
        let ids = req_event_ids(
            &state,
            &tenant,
            &outsider,
            serde_json::json!({"kinds": [9, 39000, 39001, 39002]}),
        )
        .await;
        assert!(
            ids.is_empty(),
            "global history does not expose DMs or their rosters"
        );
        assert!(
            ingest(
                &state,
                &tenant,
                message_event(&outsider, channel_id),
                &outsider
            )
            .await
            .is_err(),
            "an outsider cannot post to a DM"
        );
    }
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
