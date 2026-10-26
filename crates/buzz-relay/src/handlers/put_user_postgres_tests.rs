//! Postgres-backed tests for a kind:9000 that names several people: each is
//! checked and added on its own, and the OK message reports each result.

use std::sync::Arc;

use buzz_core::TenantContext;
use buzz_db::event::EventQuery;
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use uuid::Uuid;

use crate::handlers::group_state_postgres_tests::{
    community, create_channel, group_state_tags, http_auth, state,
};
use crate::handlers::ingest::{ingest_event, IngestError, IngestResult};
use crate::state::AppState;

fn put_user(actor: &Keys, channel_id: Uuid, p_tags: &[&[&str]], extra: &[&[&str]]) -> Event {
    let mut tags = vec![Tag::parse(["h", &channel_id.to_string()]).unwrap()];
    for tag in p_tags {
        tags.push(Tag::parse(["p"].iter().chain(tag.iter()).copied()).unwrap());
    }
    for tag in extra {
        tags.push(Tag::parse(tag.iter().copied()).unwrap());
    }
    // An actor may name themselves (stepping down, or a self-add); the
    // builder drops a `p` tag for its own key unless told not to.
    EventBuilder::new(Kind::Custom(9000), "")
        .tags(tags)
        .allow_self_tagging()
        .sign_with_keys(actor)
        .expect("sign 9000")
}

async fn ingest(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    event: Event,
    keys: &Keys,
) -> Result<IngestResult, IngestError> {
    ingest_event(state, tenant, event, http_auth(keys)).await
}

/// The JSON after the `members: ` prefix of an OK message.
fn report(message: &str) -> serde_json::Value {
    let json = message
        .strip_prefix("members: ")
        .unwrap_or_else(|| panic!("no members report: {message}"));
    serde_json::from_str(json).expect("members report JSON")
}

async fn role_of(
    state: &AppState,
    tenant: &TenantContext,
    channel_id: Uuid,
    keys: &Keys,
) -> Option<String> {
    state
        .db
        .get_members(tenant.community(), channel_id)
        .await
        .expect("members")
        .into_iter()
        .find(|member| member.pubkey == keys.public_key().to_bytes())
        .map(|member| member.role)
}

async fn refuse_third_party_adds(state: &AppState, tenant: &TenantContext, agent: &Keys) {
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
}

async fn joined_notices(
    state: &AppState,
    tenant: &TenantContext,
    channel_id: Uuid,
    target: &Keys,
) -> usize {
    state
        .db
        .query_events(&EventQuery {
            kinds: Some(vec![buzz_core::kind::KIND_SYSTEM_MESSAGE as i32]),
            channel_id: Some(channel_id),
            ..EventQuery::for_community(tenant.community())
        })
        .await
        .expect("system messages")
        .iter()
        .filter(|stored| {
            let content: serde_json::Value =
                serde_json::from_str(&stored.event.content).unwrap_or_default();
            content["type"] == "member_joined" && content["target"] == target.public_key().to_hex()
        })
        .count()
}

/// One event adds a new person, leaves an existing member alone, takes a
/// per-`p` role, and reports a refused agent without refusing the others.
/// Mutations: refuse the event on any refused person → RED; read only the
/// first `p` → later people missing → RED; ignore the `p` role → agent is a
/// member, not a bot → RED; notify a no-op re-add → two notices → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn put_user_adds_each_person_and_reports_each_result() {
    let state = state().await;
    let tenant = community(&state, "put-user-batch").await;
    let (owner, existing, person, bot, refusing) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    let channel_id = create_channel(
        &state,
        &tenant,
        &owner,
        &[
            &["visibility", "private"],
            &["p", &existing.public_key().to_hex()],
        ],
    )
    .await;
    refuse_third_party_adds(&state, &tenant, &refusing).await;

    let (existing_hex, person_hex, bot_hex, refusing_hex) = (
        existing.public_key().to_hex(),
        person.public_key().to_hex(),
        bot.public_key().to_hex(),
        refusing.public_key().to_hex(),
    );
    let result = ingest(
        &state,
        &tenant,
        put_user(
            &owner,
            channel_id,
            &[
                &[&existing_hex],
                &[&person_hex],
                &[&refusing_hex],
                &[&bot_hex, "bot"],
                &[&person_hex],
            ],
            &[],
        ),
        &owner,
    )
    .await
    .expect("put-user");
    assert!(result.accepted, "{}", result.message);
    assert_eq!(
        report(&result.message),
        serde_json::json!({
            "added": [person_hex, bot_hex],
            "already_member": [existing_hex],
            "rejected": [{
                "reason": "policy:nobody — this agent has disabled external channel additions",
                "pubkeys": [refusing_hex],
            }],
        })
    );

    assert_eq!(
        role_of(&state, &tenant, channel_id, &person)
            .await
            .as_deref(),
        Some("member")
    );
    assert_eq!(
        role_of(&state, &tenant, channel_id, &bot).await.as_deref(),
        Some("bot")
    );
    assert_eq!(role_of(&state, &tenant, channel_id, &refusing).await, None);
    assert_eq!(
        joined_notices(&state, &tenant, channel_id, &person).await,
        1
    );
    assert_eq!(
        joined_notices(&state, &tenant, channel_id, &existing).await,
        1
    );

    let roster = group_state_tags(&state, &tenant, channel_id, 39002).await;
    for hex in [&person_hex, &bot_hex] {
        assert!(
            roster.iter().any(|tag| tag[0] == "p" && &tag[1] == hex),
            "{roster:?}"
        );
    }
}

/// A role change on an active member is reported as `updated`; the event's
/// `role` tag applies where a `p` has none.
/// Mutation: report every active member as already_member → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn put_user_reports_role_changes_as_updated() {
    let state = state().await;
    let tenant = community(&state, "put-user-updated").await;
    let (owner, existing, person) = (Keys::generate(), Keys::generate(), Keys::generate());
    let channel_id = create_channel(
        &state,
        &tenant,
        &owner,
        &[&["p", &existing.public_key().to_hex()]],
    )
    .await;

    let (existing_hex, person_hex) = (existing.public_key().to_hex(), person.public_key().to_hex());
    let result = ingest(
        &state,
        &tenant,
        put_user(
            &owner,
            channel_id,
            &[&[&existing_hex], &[&person_hex, ""]],
            &[&["role", "admin"]],
        ),
        &owner,
    )
    .await
    .expect("put-user");
    assert_eq!(
        report(&result.message),
        serde_json::json!({ "added": [person_hex], "updated": [existing_hex] })
    );
    assert_eq!(
        role_of(&state, &tenant, channel_id, &existing)
            .await
            .as_deref(),
        Some("admin")
    );
    assert_eq!(
        role_of(&state, &tenant, channel_id, &person)
            .await
            .as_deref(),
        Some("admin")
    );
}

/// When nobody named may be added the event is refused: one person keeps the
/// single-add error, several get the grouped report. A malformed `p` refuses
/// the whole event.
/// Mutations: accept an all-refused event → RED; report a single refusal as
/// JSON → RED; skip malformed `p` tags → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn put_user_refuses_the_event_when_nobody_can_be_added() {
    let state = state().await;
    let tenant = community(&state, "put-user-refused").await;
    let (owner, first, second) = (Keys::generate(), Keys::generate(), Keys::generate());
    let channel_id = create_channel(&state, &tenant, &owner, &[]).await;
    refuse_third_party_adds(&state, &tenant, &first).await;
    refuse_third_party_adds(&state, &tenant, &second).await;
    let (first_hex, second_hex) = (first.public_key().to_hex(), second.public_key().to_hex());

    let refusal = |result: Result<IngestResult, IngestError>| match result {
        Err(IngestError::Rejected(message)) => message,
        Err(error) => panic!("expected a rejection, got {error:?}"),
        Ok(result) => panic!("expected a rejection, got OK {}", result.message),
    };

    let single = refusal(
        ingest(
            &state,
            &tenant,
            put_user(&owner, channel_id, &[&[&first_hex]], &[]),
            &owner,
        )
        .await,
    );
    assert_eq!(
        single,
        "invalid: policy:nobody — this agent has disabled external channel additions"
    );

    let several = refusal(
        ingest(
            &state,
            &tenant,
            put_user(&owner, channel_id, &[&[&first_hex], &[&second_hex]], &[]),
            &owner,
        )
        .await,
    );
    let json = several.strip_prefix("invalid: ").expect("invalid prefix");
    assert_eq!(
        report(json),
        serde_json::json!({
            "rejected": [{
                "reason": "policy:nobody — this agent has disabled external channel additions",
                "pubkeys": [first_hex, second_hex],
            }],
        })
    );

    let malformed = refusal(
        ingest(
            &state,
            &tenant,
            put_user(&owner, channel_id, &[&["not-hex"], &[&second_hex]], &[]),
            &owner,
        )
        .await,
    );
    assert_eq!(malformed, "invalid: p tag must be a 32-byte hex pubkey");
}

/// Each person gets the checks a single add would: a plain member in an open
/// channel may add someone, but not grant one of them an elevated role.
/// Mutations: check the event-wide role instead of each `p` role → RED; report
/// JSON for a single add → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn put_user_checks_each_persons_role_for_the_actor() {
    let state = state().await;
    let tenant = community(&state, "put-user-member-actor").await;
    let (owner, member, promoted, plain) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    let channel_id = create_channel(
        &state,
        &tenant,
        &owner,
        &[
            &["visibility", "open"],
            &["p", &member.public_key().to_hex()],
        ],
    )
    .await;

    let (promoted_hex, plain_hex) = (promoted.public_key().to_hex(), plain.public_key().to_hex());
    let result = ingest(
        &state,
        &tenant,
        put_user(
            &member,
            channel_id,
            &[&[&promoted_hex, "admin"], &[&plain_hex]],
            &[],
        ),
        &member,
    )
    .await
    .expect("put-user");
    assert_eq!(
        report(&result.message),
        serde_json::json!({
            "added": [plain_hex],
            "rejected": [{
                "reason": "only owners/admins may grant elevated roles",
                "pubkeys": [promoted_hex],
            }],
        })
    );
    assert_eq!(role_of(&state, &tenant, channel_id, &promoted).await, None);

    // One person keeps a single add's empty OK message.
    let single = ingest(
        &state,
        &tenant,
        put_user(&member, channel_id, &[&[&promoted_hex]], &[]),
        &member,
    )
    .await
    .expect("single put-user");
    assert!(single.accepted);
    assert_eq!(single.message, "");
    assert_eq!(
        role_of(&state, &tenant, channel_id, &promoted)
            .await
            .as_deref(),
        Some("member")
    );
}

/// One event may name at most 100 people.
/// Mutation: drop the cap → 101 accepted → RED; cap below 100 → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn put_user_names_at_most_one_hundred_people() {
    let state = state().await;
    let tenant = community(&state, "put-user-cap").await;
    let owner = Keys::generate();
    let channel_id = create_channel(&state, &tenant, &owner, &[]).await;
    let people: Vec<String> = (0..101)
        .map(|_| Keys::generate().public_key().to_hex())
        .collect();
    let tags = |count: usize| -> Vec<Vec<&str>> {
        people[..count]
            .iter()
            .map(|hex| vec![hex.as_str()])
            .collect()
    };
    let event = |count: usize| {
        let tags = tags(count);
        let refs: Vec<&[&str]> = tags.iter().map(Vec::as_slice).collect();
        put_user(&owner, channel_id, &refs, &[])
    };

    match ingest(&state, &tenant, event(101), &owner).await {
        Err(IngestError::Rejected(message)) => {
            assert_eq!(message, "invalid: a put-user may name at most 100 people")
        }
        Err(error) => panic!("expected a rejection, got {error:?}"),
        Ok(result) => panic!("expected a rejection, got OK {}", result.message),
    }

    let result = ingest(&state, &tenant, event(100), &owner)
        .await
        .expect("put-user");
    assert_eq!(
        report(&result.message)["added"].as_array().map(Vec::len),
        Some(100)
    );
}

/// The relay decides people in tag order, each against the roster the people
/// before them left, as `put_members` writes them. A sole owner who names an
/// heir first may then step down; stepping down first is refused, since no
/// heir exists yet; an heir refused by their add policy is never counted.
/// Mutations: decide everyone against the roster before the event → the
/// heir-first owner stays owner → RED; count a policy-refused heir → the
/// owner steps down with no heir → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn put_user_decides_each_person_after_the_ones_before() {
    let state = state().await;
    let tenant = community(&state, "put-user-ordered").await;
    let (owner, heir, refusing) = (Keys::generate(), Keys::generate(), Keys::generate());
    refuse_third_party_adds(&state, &tenant, &refusing).await;
    let (owner_hex, heir_hex, refusing_hex) = (
        owner.public_key().to_hex(),
        heir.public_key().to_hex(),
        refusing.public_key().to_hex(),
    );
    let last_owner = "cannot demote the last owner — transfer ownership first";

    // Heir first: ownership moves in one event.
    let channel_id = create_channel(&state, &tenant, &owner, &[]).await;
    let result = ingest(
        &state,
        &tenant,
        put_user(
            &owner,
            channel_id,
            &[&[&heir_hex, "owner"], &[&owner_hex, "member"]],
            &[],
        ),
        &owner,
    )
    .await
    .expect("heir first");
    assert_eq!(
        report(&result.message),
        serde_json::json!({ "added": [heir_hex], "updated": [owner_hex] })
    );
    assert_eq!(
        role_of(&state, &tenant, channel_id, &heir).await.as_deref(),
        Some("owner")
    );
    assert_eq!(
        role_of(&state, &tenant, channel_id, &owner)
            .await
            .as_deref(),
        Some("member")
    );

    // Step down first: refused, the heir is still added.
    let channel_id = create_channel(&state, &tenant, &owner, &[]).await;
    let result = ingest(
        &state,
        &tenant,
        put_user(
            &owner,
            channel_id,
            &[&[&owner_hex, "member"], &[&heir_hex, "owner"]],
            &[],
        ),
        &owner,
    )
    .await
    .expect("step down first");
    assert_eq!(
        report(&result.message),
        serde_json::json!({
            "added": [heir_hex],
            "rejected": [{ "reason": last_owner, "pubkeys": [owner_hex] }],
        })
    );
    assert_eq!(
        role_of(&state, &tenant, channel_id, &owner)
            .await
            .as_deref(),
        Some("owner")
    );

    // A policy-refused heir doesn't count: nobody can be added.
    let channel_id = create_channel(&state, &tenant, &owner, &[]).await;
    let refused = match ingest(
        &state,
        &tenant,
        put_user(
            &owner,
            channel_id,
            &[&[&refusing_hex, "owner"], &[&owner_hex, "member"]],
            &[],
        ),
        &owner,
    )
    .await
    {
        Err(IngestError::Rejected(message)) => message,
        Err(error) => panic!("expected a rejection, got {error:?}"),
        Ok(result) => panic!("expected a rejection, got OK {}", result.message),
    };
    let json = refused.strip_prefix("invalid: ").expect("invalid prefix");
    assert_eq!(
        report(json),
        serde_json::json!({
            "rejected": [
                {
                    "reason": "policy:nobody — this agent has disabled external channel additions",
                    "pubkeys": [refusing_hex],
                },
                { "reason": last_owner, "pubkeys": [owner_hex] },
            ],
        })
    );
    assert_eq!(
        role_of(&state, &tenant, channel_id, &owner)
            .await
            .as_deref(),
        Some("owner")
    );
}
