//! Postgres-backed tests for growing a DM: kind:41011 adding people in place
//! (`["history", "include"]`) or into a new DM, agents joining without
//! changing which DM it is, and kind:41010 finding a DM by its people.

use std::sync::Arc;

use buzz_core::TenantContext;
use nostr::{EventBuilder, Keys, Kind, Tag};
use uuid::Uuid;

use super::group_state_postgres_tests::{community, http_auth, state};
use crate::handlers::ingest::{ingest_event, IngestError, IngestResult};
use crate::state::AppState;

fn hex(keys: &Keys) -> String {
    keys.public_key().to_hex()
}

async fn submit(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    actor: &Keys,
    kind: u16,
    tags: Vec<Vec<String>>,
) -> Result<IngestResult, IngestError> {
    let tags: Vec<Tag> = tags
        .into_iter()
        .map(|tag| Tag::parse(tag).expect("tag"))
        // A unique tag so identical commands are distinct events.
        .chain([Tag::parse(["client", &Uuid::new_v4().to_string()]).expect("tag")])
        .collect();
    let event = EventBuilder::new(Kind::Custom(kind), "")
        .tags(tags)
        .sign_with_keys(actor)
        .expect("sign");
    ingest_event(state, tenant, event, http_auth(actor)).await
}

fn response_channel(result: &IngestResult) -> (Uuid, bool) {
    let json: serde_json::Value = serde_json::from_str(
        result
            .message
            .strip_prefix("response:")
            .unwrap_or_else(|| panic!("no response in {:?}", result.message)),
    )
    .expect("response json");
    (
        json["channel_id"]
            .as_str()
            .expect("channel_id")
            .parse()
            .expect("uuid"),
        json["created"].as_bool().expect("created"),
    )
}

async fn open(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    actor: &Keys,
    others: &[&Keys],
) -> Result<Uuid, IngestError> {
    open_tagged(state, tenant, actor, others, false).await
}

/// Open a DM with `others`, agreeing to share an existing DM's history.
async fn open_with_history(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    actor: &Keys,
    others: &[&Keys],
) -> Result<Uuid, IngestError> {
    open_tagged(state, tenant, actor, others, true).await
}

async fn open_tagged(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    actor: &Keys,
    others: &[&Keys],
    history: bool,
) -> Result<Uuid, IngestError> {
    let mut tags: Vec<Vec<String>> = others.iter().map(|k| vec!["p".into(), hex(k)]).collect();
    if history {
        tags.push(vec!["history".into(), "include".into()]);
    }
    submit(state, tenant, actor, 41010, tags)
        .await
        .map(|r| response_channel(&r).0)
}

async fn add(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    actor: &Keys,
    dm: Uuid,
    others: &[&Keys],
    include_history: bool,
) -> Result<(Uuid, bool), IngestError> {
    let mut tags: Vec<Vec<String>> = vec![vec!["h".into(), dm.to_string()]];
    tags.extend(others.iter().map(|k| vec!["p".into(), hex(k)]));
    if include_history {
        tags.push(vec!["history".into(), "include".into()]);
    }
    submit(state, tenant, actor, 41011, tags)
        .await
        .map(|r| response_channel(&r))
}

async fn members(state: &AppState, tenant: &TenantContext, dm: Uuid) -> Vec<String> {
    let mut members: Vec<String> = state
        .db
        .get_members(tenant.community(), dm)
        .await
        .expect("members")
        .into_iter()
        .map(|m| ::hex::encode(m.pubkey))
        .collect();
    members.sort();
    members
}

fn set_of(keys: &[&Keys]) -> Vec<String> {
    let mut set: Vec<String> = keys.iter().map(|k| hex(k)).collect();
    set.sort();
    set
}

/// Make `agent` an agent owned by `owner`, with `policy` for third-party adds.
async fn make_agent(
    state: &AppState,
    tenant: &TenantContext,
    agent: &Keys,
    owner: &Keys,
    policy: &str,
) {
    let agent_bytes = agent.public_key().to_bytes();
    let owner_bytes = owner.public_key().to_bytes();
    for pk in [&agent_bytes, &owner_bytes] {
        state
            .db
            .ensure_user(tenant.community(), pk)
            .await
            .expect("user");
    }
    state
        .db
        .set_agent_owner(tenant.community(), &agent_bytes, &owner_bytes)
        .await
        .expect("owner");
    state
        .db
        .set_channel_add_policy(tenant.community(), &agent_bytes, policy)
        .await
        .expect("policy");
}

fn rejected(result: Result<impl std::fmt::Debug, IngestError>, needle: &str) {
    match result {
        Err(IngestError::Rejected(message)) => {
            assert!(message.contains(needle), "{message:?} lacks {needle:?}")
        }
        other => panic!("expected a refusal containing {needle:?}, got {other:?}"),
    }
}

/// Adding a person with history keeps the DM and moves its identity to the
/// grown set of people: the original people alone now get a new DM, and the
/// grown set finds this one.
/// Mutations: fork instead of growing → a different channel → RED; leave the
/// fingerprint behind → opening {A, B} finds the grown DM → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn adding_a_person_with_history_grows_the_dm_in_place() {
    let state = state().await;
    let tenant = community(&state, "dm-grow-in-place").await;
    let (a, b, c) = (Keys::generate(), Keys::generate(), Keys::generate());
    let dm = open(&state, &tenant, &a, &[&b]).await.expect("open");

    let (target, created) = add(&state, &tenant, &a, dm, &[&c], true)
        .await
        .expect("add");

    assert_eq!((target, created), (dm, false));
    assert_eq!(members(&state, &tenant, dm).await, set_of(&[&a, &b, &c]));
    assert_eq!(
        open(&state, &tenant, &b, &[&a, &c]).await.expect("reopen"),
        dm
    );
    let fresh = open(&state, &tenant, &a, &[&b])
        .await
        .expect("open original people");
    assert_ne!(fresh, dm, "the original people start a new DM");
    assert_eq!(members(&state, &tenant, fresh).await, set_of(&[&a, &b]));
}

/// Without the history tag, adding a person starts a new DM of everyone and
/// leaves the original untouched, as before.
/// Mutation: treat a missing tag as include → the original grows → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn adding_a_person_without_history_starts_a_new_dm() {
    let state = state().await;
    let tenant = community(&state, "dm-grow-fork").await;
    let (a, b, c) = (Keys::generate(), Keys::generate(), Keys::generate());
    let dm = open(&state, &tenant, &a, &[&b]).await.expect("open");

    let (target, created) = add(&state, &tenant, &a, dm, &[&c], false)
        .await
        .expect("add");

    assert!(created);
    assert_ne!(target, dm);
    assert_eq!(members(&state, &tenant, dm).await, set_of(&[&a, &b]));
    assert_eq!(
        members(&state, &tenant, target).await,
        set_of(&[&a, &b, &c])
    );
    assert_eq!(open(&state, &tenant, &a, &[&b]).await.expect("reopen"), dm);
}

/// When a DM of the grown people already exists, growing returns it and
/// changes nothing.
/// Mutation: drop the collision check → the unique index refuses the move, or
/// the DM grows anyway → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn growing_into_an_existing_group_returns_that_dm() {
    let state = state().await;
    let tenant = community(&state, "dm-grow-collision").await;
    let (a, b, c) = (Keys::generate(), Keys::generate(), Keys::generate());
    let pair = open(&state, &tenant, &a, &[&b]).await.expect("open pair");
    let trio = open(&state, &tenant, &a, &[&b, &c])
        .await
        .expect("open trio");

    let (target, created) = add(&state, &tenant, &a, pair, &[&c], true)
        .await
        .expect("add");

    assert_eq!((target, created), (trio, false));
    assert_eq!(members(&state, &tenant, pair).await, set_of(&[&a, &b]));
    assert_eq!(
        open(&state, &tenant, &a, &[&b]).await.expect("reopen"),
        pair
    );
}

/// With the history tag, an agent joins in place and never changes which DM it
/// is: the same people still find it. Without it, the agent starts a new DM of
/// everyone, as before, and this DM is untouched: an older client cannot share
/// a DM's history unseen.
/// Mutations: fork on agents with the tag → a different channel → RED; join
/// in place without it → the same channel → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_agent_joins_in_place_without_changing_the_dm() {
    let state = state().await;
    let tenant = community(&state, "dm-grow-agent").await;
    let (a, b, agent) = (Keys::generate(), Keys::generate(), Keys::generate());
    make_agent(&state, &tenant, &agent, &a, "owner_only").await;
    let dm = open(&state, &tenant, &a, &[&b]).await.expect("open");

    let (forked, created) = add(&state, &tenant, &a, dm, &[&agent], false)
        .await
        .expect("add without history");
    assert!(created);
    assert_ne!(forked, dm);
    assert_eq!(members(&state, &tenant, dm).await, set_of(&[&a, &b]));

    let (target, created) = add(&state, &tenant, &a, dm, &[&agent], true)
        .await
        .expect("add with history");

    assert_eq!((target, created), (dm, false));
    assert_eq!(
        members(&state, &tenant, dm).await,
        set_of(&[&a, &b, &agent])
    );
    assert_eq!(open(&state, &tenant, &b, &[&a]).await.expect("reopen"), dm);
}

/// Opening a DM with people and an agent, with the history tag, finds the
/// people's DM and adds the agent to it. Without the tag it finds or makes the
/// DM of exactly those members, as before, and the people's DM is untouched.
/// Mutations: skip adding named agents to a found DM → agent missing → RED;
/// find the people's DM without the tag → the same channel → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn opening_with_an_agent_finds_the_peoples_dm_and_adds_it() {
    let state = state().await;
    let tenant = community(&state, "dm-open-agent").await;
    let (a, b, agent) = (Keys::generate(), Keys::generate(), Keys::generate());
    make_agent(&state, &tenant, &agent, &a, "anyone").await;
    let dm = open(&state, &tenant, &a, &[&b]).await.expect("open");

    let separate = open(&state, &tenant, &a, &[&b, &agent])
        .await
        .expect("open without history");
    assert_ne!(separate, dm);
    assert_eq!(members(&state, &tenant, dm).await, set_of(&[&a, &b]));
    assert_eq!(
        open(&state, &tenant, &a, &[&b, &agent])
            .await
            .expect("again without history"),
        separate
    );

    assert_eq!(
        open_with_history(&state, &tenant, &a, &[&b, &agent])
            .await
            .expect("open with history"),
        dm
    );
    assert_eq!(
        members(&state, &tenant, dm).await,
        set_of(&[&a, &b, &agent])
    );
}

/// A DM of someone and their agents is still identified by all its members,
/// so each agent keeps its own DM with its owner.
/// Mutation: identify one-person DMs by people only → both agents share one
/// DM → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn each_agent_keeps_its_own_dm_with_its_owner() {
    let state = state().await;
    let tenant = community(&state, "dm-own-agents").await;
    let (a, first, second) = (Keys::generate(), Keys::generate(), Keys::generate());
    make_agent(&state, &tenant, &first, &a, "owner_only").await;
    make_agent(&state, &tenant, &second, &a, "owner_only").await;

    let dm_first = open(&state, &tenant, &a, &[&first]).await.expect("first");
    let dm_second = open(&state, &tenant, &a, &[&second]).await.expect("second");

    assert_ne!(dm_first, dm_second);
    assert_eq!(
        open(&state, &tenant, &a, &[&first]).await.expect("again"),
        dm_first
    );
}

/// The nine-person limit counts people only.
/// Mutations: count agents toward the limit → the agent add is refused → RED;
/// drop the limit → a tenth person joins → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn the_limit_counts_people_not_agents() {
    let state = state().await;
    let tenant = community(&state, "dm-limit").await;
    let a = Keys::generate();
    let others: Vec<Keys> = (0..8).map(|_| Keys::generate()).collect();
    let other_refs: Vec<&Keys> = others.iter().collect();
    let agent = Keys::generate();
    make_agent(&state, &tenant, &agent, &a, "anyone").await;
    let dm = open(&state, &tenant, &a, &other_refs)
        .await
        .expect("nine people");

    add(&state, &tenant, &a, dm, &[&agent], true)
        .await
        .expect("agent beyond nine");
    let tenth = Keys::generate();
    rejected(
        add(&state, &tenant, &a, dm, &[&tenth], true).await,
        "at most 9 people",
    );
    rejected(
        add(&state, &tenant, &a, dm, &[&tenth], false).await,
        "at most 9 people",
    );
}

/// Adding someone else's agent follows its `channel_add_policy`.
/// Mutation: skip the policy check → the agent joins → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn adding_an_agent_follows_its_add_policy() {
    let state = state().await;
    let tenant = community(&state, "dm-agent-policy").await;
    let (a, b, agent) = (Keys::generate(), Keys::generate(), Keys::generate());
    make_agent(&state, &tenant, &agent, &b, "owner_only").await;
    let dm = open(&state, &tenant, &a, &[&b]).await.expect("open");

    rejected(
        add(&state, &tenant, &a, dm, &[&agent], true).await,
        "policy:owner_only",
    );
    rejected(
        open(&state, &tenant, &a, &[&b, &agent]).await,
        "policy:owner_only",
    );
    assert_eq!(members(&state, &tenant, dm).await, set_of(&[&a, &b]));

    let (target, _) = add(&state, &tenant, &b, dm, &[&agent], true)
        .await
        .expect("owner adds");
    assert_eq!(target, dm);
}

/// An agent naming its owner's people cannot open or join their DM: it gets
/// its own DM with them, identified by all its members.
/// Mutation: return a found DM to a non-member, or identify an agent's DM by
/// its people → the agent gets the people's DM → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_agent_cannot_open_its_owners_dm_with_others() {
    let state = state().await;
    let tenant = community(&state, "dm-agent-actor").await;
    let (a, b, agent) = (Keys::generate(), Keys::generate(), Keys::generate());
    make_agent(&state, &tenant, &agent, &a, "anyone").await;
    let dm = open(&state, &tenant, &a, &[&b]).await.expect("open");

    let own = open(&state, &tenant, &agent, &[&a, &b])
        .await
        .expect("agent open");

    assert_ne!(own, dm);
    assert_eq!(members(&state, &tenant, dm).await, set_of(&[&a, &b]));
}

/// A DM an agent starts with people does not become those people's DM, and
/// they still find it when they name the agent too.
/// Mutation: identify an agent's DM by its people → B finds it without the
/// agent → RED; drop the all-members lookup → B naming the agent forks → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_agents_group_dm_keeps_its_own_identity() {
    let state = state().await;
    let tenant = community(&state, "dm-agent-group").await;
    let (owner, b, c, agent) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    make_agent(&state, &tenant, &agent, &owner, "anyone").await;
    let agents_dm = open(&state, &tenant, &agent, &[&b, &c])
        .await
        .expect("agent open");

    assert_eq!(
        open(&state, &tenant, &b, &[&c, &agent])
            .await
            .expect("open with agent"),
        agents_dm
    );
    let peoples = open(&state, &tenant, &b, &[&c]).await.expect("people open");
    assert_ne!(peoples, agents_dm);
}

/// Only people share a DM's history: an agent's add or open with the tag is
/// refused, and an add without it starts a new DM, as before.
/// Mutation: let agents grow DMs → C joins the original DM → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_agent_cannot_share_a_dms_history() {
    let state = state().await;
    let tenant = community(&state, "dm-agent-history").await;
    let (a, b, c, agent) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    make_agent(&state, &tenant, &agent, &a, "anyone").await;
    let dm = open(&state, &tenant, &a, &[&b]).await.expect("open");
    add(&state, &tenant, &a, dm, &[&agent], true)
        .await
        .expect("add agent");

    rejected(
        add(&state, &tenant, &agent, dm, &[&c], true).await,
        "only people",
    );
    rejected(
        open_with_history(&state, &tenant, &agent, &[&a, &b]).await,
        "only people",
    );
    let (target, created) = add(&state, &tenant, &agent, dm, &[&c], false)
        .await
        .expect("agent add");

    assert!(created);
    assert_ne!(target, dm);
    assert_eq!(
        members(&state, &tenant, dm).await,
        set_of(&[&a, &b, &agent])
    );
}

/// Growing a DM never reveals another DM the actor is not in.
/// Mutation: return `Existing` without checking membership → the agent
/// learns the people's DM → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn growing_does_not_reveal_a_dm_the_actor_is_not_in() {
    let state = state().await;
    let tenant = community(&state, "dm-grow-reveal").await;
    let (a, b, c, agent) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    make_agent(&state, &tenant, &agent, &a, "anyone").await;
    let dm = open(&state, &tenant, &a, &[&b]).await.expect("open");
    add(&state, &tenant, &a, dm, &[&agent], true)
        .await
        .expect("add agent");
    open(&state, &tenant, &a, &[&b, &c]).await.expect("trio");

    let c_bytes = c.public_key().to_bytes();
    let result = state
        .db
        .grow_dm(
            tenant.community(),
            dm,
            &agent.public_key().to_bytes(),
            &[c_bytes.as_slice()],
            &|_: &str, _: &[u8]| Ok(()),
            false,
        )
        .await;

    assert!(
        matches!(result, Err(buzz_db::DbError::AccessDenied(_))),
        "{result:?}"
    );
}

/// The relay's moderation DM stays two-party, whether added to or opened with
/// someone else.
/// Mutation: drop either relay-key guard → a member joins → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn the_moderation_dm_cannot_grow() {
    let state = state().await;
    let tenant = community(&state, "dm-moderation").await;
    let (a, b) = (Keys::generate(), Keys::generate());
    let relay = state.relay_keypair.public_key().to_bytes();
    let (dm, _) = state
        .db
        .open_dm(
            tenant.community(),
            &[&a.public_key().to_bytes()],
            &relay,
            false,
            &|_: &str, _: &[u8]| Ok(()),
        )
        .await
        .expect("moderation dm");

    rejected(
        add(&state, &tenant, &a, dm.id, &[&b], true).await,
        "cannot be changed",
    );
    rejected(
        add(&state, &tenant, &a, dm.id, &[&b], false).await,
        "cannot be changed",
    );
    let agent = Keys::generate();
    make_agent(&state, &tenant, &agent, &a, "anyone").await;
    rejected(
        open(&state, &tenant, &a, &[&state.relay_keypair, &agent]).await,
        "cannot be changed",
    );
    assert_eq!(
        members(&state, &tenant, dm.id).await,
        set_of(&[&a, &state.relay_keypair])
    );
}

/// Only `include` is a valid history value.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_unknown_history_value_is_refused() {
    let state = state().await;
    let tenant = community(&state, "dm-history-value").await;
    let (a, b, c) = (Keys::generate(), Keys::generate(), Keys::generate());
    let dm = open(&state, &tenant, &a, &[&b]).await.expect("open");
    let tags = vec![
        vec!["h".into(), dm.to_string()],
        vec!["p".into(), hex(&c)],
        vec!["history".into(), "exclude".into()],
    ];
    rejected(
        submit(&state, &tenant, &a, 41011, tags).await.map(|_| ()),
        "history",
    );
}

/// A DM of just the caller and someone else's agent needs no add policy, as
/// before: anyone may message an agent.
/// Mutation: check the policy on every open → the DM is refused → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn messaging_an_agent_alone_ignores_its_add_policy() {
    let state = state().await;
    let tenant = community(&state, "dm-agent-alone").await;
    let (a, b, agent) = (Keys::generate(), Keys::generate(), Keys::generate());
    make_agent(&state, &tenant, &agent, &b, "nobody").await;

    let dm = open(&state, &tenant, &a, &[&agent]).await.expect("open");
    assert_eq!(members(&state, &tenant, dm).await, set_of(&[&a, &agent]));
}

/// People join a DM only through kind:41011, so a plain add, and removing
/// someone other than yourself or your own agent, are refused.
/// Mutation: drop either DM guard → the add or the removal lands → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn generic_membership_kinds_cannot_change_a_dm() {
    let state = state().await;
    let tenant = community(&state, "dm-generic-membership").await;
    let (a, b, c) = (Keys::generate(), Keys::generate(), Keys::generate());
    let dm = open(&state, &tenant, &a, &[&b]).await.expect("open");
    let h = vec!["h".to_string(), dm.to_string()];

    rejected(
        submit(
            &state,
            &tenant,
            &a,
            9000,
            vec![h.clone(), vec!["p".into(), hex(&c)]],
        )
        .await
        .map(|r| (r.accepted, r.message)),
        "kind:41011",
    );
    rejected(
        submit(
            &state,
            &tenant,
            &a,
            9001,
            vec![h, vec!["p".into(), hex(&b)]],
        )
        .await
        .map(|r| (r.accepted, r.message)),
        "kind:41011",
    );

    assert_eq!(members(&state, &tenant, dm).await, set_of(&[&a, &b]));
    assert_eq!(open(&state, &tenant, &b, &[&a]).await.expect("reopen"), dm);
}

/// Anyone can leave a DM. The DM then stops being that group's DM, so the
/// person who left can message the group again, in a new DM.
/// Mutation: refuse kind:9022 on DMs → the leave is refused → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_person_can_leave_a_dm() {
    let state = state().await;
    let tenant = community(&state, "dm-leave").await;
    let (a, b, c) = (Keys::generate(), Keys::generate(), Keys::generate());
    let dm = open(&state, &tenant, &a, &[&b, &c]).await.expect("open");

    let left = submit(
        &state,
        &tenant,
        &c,
        9022,
        vec![vec!["h".into(), dm.to_string()]],
    )
    .await
    .expect("leave");
    assert!(left.accepted, "{}", left.message);

    assert_eq!(members(&state, &tenant, dm).await, set_of(&[&a, &b]));
    let again = open(&state, &tenant, &c, &[&a, &b])
        .await
        .expect("message the group again");
    assert_ne!(again, dm);
}

/// An owner can remove their own agent from a DM, as deleting an agent does;
/// the DM stays its people's DM. Someone else's agent cannot be removed.
/// Mutation: refuse every kind:9001 for another member on DMs → the owner's
/// removal is refused → RED; allow any → B removes A's agent → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_owner_can_remove_their_agent_from_a_dm() {
    let state = state().await;
    let tenant = community(&state, "dm-remove-agent").await;
    let (a, b, agent) = (Keys::generate(), Keys::generate(), Keys::generate());
    make_agent(&state, &tenant, &agent, &a, "anyone").await;
    let dm = open_with_history(&state, &tenant, &a, &[&b, &agent])
        .await
        .expect("open");
    let remove = vec![
        vec!["h".to_string(), dm.to_string()],
        vec!["p".into(), hex(&agent)],
    ];

    rejected(
        submit(&state, &tenant, &b, 9001, remove.clone())
            .await
            .map(|r| (r.accepted, r.message)),
        "kind:41011",
    );
    let removed = submit(&state, &tenant, &a, 9001, remove)
        .await
        .expect("remove");
    assert!(removed.accepted, "{}", removed.message);

    assert_eq!(members(&state, &tenant, dm).await, set_of(&[&a, &b]));
    assert_eq!(open(&state, &tenant, &b, &[&a]).await.expect("reopen"), dm);
}

/// A moderation kick from a DM still removes the member, and the DM stops
/// being any group's DM: the person removed, like the rest, can message that
/// group again, in a new DM.
/// Mutation: keep the DM's identity after the kick → the removed person is
/// refused ("already exists") → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_kick_from_a_dm_releases_its_identity() {
    let state = state().await;
    let tenant = community(&state, "dm-kick").await;
    let (a, b, c) = (Keys::generate(), Keys::generate(), Keys::generate());
    let dm = open(&state, &tenant, &a, &[&b, &c]).await.expect("open");

    state
        .db
        .deploy_kick_member(
            tenant.community(),
            dm,
            &c.public_key().to_bytes(),
            &a.public_key().to_bytes(),
        )
        .await
        .expect("kick");

    assert_eq!(members(&state, &tenant, dm).await, set_of(&[&a, &b]));
    let again = open(&state, &tenant, &c, &[&a, &b])
        .await
        .expect("message the group again");
    assert_ne!(again, dm);
}

/// Agents are not part of a DM of people's identity, so kicking one leaves
/// the DM as that group's DM; kicking an agent from a DM of someone and
/// their agent releases it, since all its members identify it.
/// Mutation: release on every removal → the group gets a new DM → RED;
/// never release when an agent is kicked → the owner's reopen finds the
/// emptied agent DM → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn kicking_an_agent_keeps_a_group_dm_its_identity() {
    let state = state().await;
    let tenant = community(&state, "dm-kick-agent").await;
    let (a, b, agent) = (Keys::generate(), Keys::generate(), Keys::generate());
    make_agent(&state, &tenant, &agent, &a, "anyone").await;
    let group = open_with_history(&state, &tenant, &a, &[&b, &agent])
        .await
        .expect("group");
    let own = open(&state, &tenant, &a, &[&agent]).await.expect("own");

    for dm in [group, own] {
        state
            .db
            .deploy_kick_member(
                tenant.community(),
                dm,
                &agent.public_key().to_bytes(),
                &a.public_key().to_bytes(),
            )
            .await
            .expect("kick");
    }

    assert_eq!(
        open(&state, &tenant, &b, &[&a]).await.expect("reopen"),
        group
    );
    assert_ne!(
        open(&state, &tenant, &a, &[&agent])
            .await
            .expect("own again"),
        own
    );
}

/// Opening a DM that puts someone else's agent with anyone besides the caller
/// follows its add policy, even when no other person is in it.
/// Mutation: check policies only when people are named → the DM opens → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn opening_with_several_agents_follows_their_add_policy() {
    let state = state().await;
    let tenant = community(&state, "dm-agents-policy").await;
    let (a, b) = (Keys::generate(), Keys::generate());
    let (theirs, mine) = (Keys::generate(), Keys::generate());
    make_agent(&state, &tenant, &theirs, &b, "nobody").await;
    make_agent(&state, &tenant, &mine, &a, "owner_only").await;

    rejected(
        open(&state, &tenant, &a, &[&theirs, &mine]).await,
        "policy:nobody",
    );
}

/// Adding an agent to a DM of one person and their agent moves its identity
/// to all its members: the original pair gets a new DM.
/// Mutation: keep the old identity → reopening the pair finds the grown DM →
/// RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_agent_joining_a_one_person_dm_moves_its_identity() {
    let state = state().await;
    let tenant = community(&state, "dm-one-person-grow").await;
    let (a, b) = (Keys::generate(), Keys::generate());
    let (mine, theirs) = (Keys::generate(), Keys::generate());
    make_agent(&state, &tenant, &mine, &a, "anyone").await;
    make_agent(&state, &tenant, &theirs, &b, "anyone").await;
    let dm = open(&state, &tenant, &a, &[&mine]).await.expect("open");

    let (target, _) = add(&state, &tenant, &a, dm, &[&theirs], true)
        .await
        .expect("add agent");
    assert_eq!(target, dm);
    assert_eq!(
        members(&state, &tenant, dm).await,
        set_of(&[&a, &mine, &theirs])
    );
    assert_ne!(
        open(&state, &tenant, &a, &[&mine]).await.expect("reopen"),
        dm
    );
    assert_eq!(
        open(&state, &tenant, &a, &[&mine, &theirs])
            .await
            .expect("grown"),
        dm
    );
}

/// When the grown identity is taken, an agent still joins a one-person DM in
/// place, and the DM stops answering to its old members, so reopening them
/// starts a new DM instead of one that now holds the agent.
/// Mutation: keep the old fingerprint on a lost claim → reopening A+X returns
/// the DM with Y in it → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_agent_joining_a_one_person_dm_whose_grown_identity_is_taken_drops_its_identity() {
    let state = state().await;
    let tenant = community(&state, "dm-one-person-grow-taken").await;
    let (a, x, y) = (Keys::generate(), Keys::generate(), Keys::generate());
    make_agent(&state, &tenant, &x, &a, "anyone").await;
    make_agent(&state, &tenant, &y, &a, "anyone").await;
    let both = open(&state, &tenant, &a, &[&x, &y])
        .await
        .expect("open both");
    let dm = open(&state, &tenant, &a, &[&x]).await.expect("open one");

    let (target, created) = add(&state, &tenant, &a, dm, &[&y], true)
        .await
        .expect("add agent");

    assert_eq!((target, created), (dm, false));
    assert_eq!(members(&state, &tenant, dm).await, set_of(&[&a, &x, &y]));
    assert_ne!(open(&state, &tenant, &a, &[&x]).await.expect("reopen"), dm);
    assert_eq!(
        open(&state, &tenant, &a, &[&x, &y])
            .await
            .expect("both again"),
        both
    );
}

/// A DM of one person and their agents never gains a person in place, even
/// with the history tag: adding one starts a DM of its people, and adding them
/// again opens that DM. The original keeps its members and its identity.
/// Mutation: let the tag grow it in place → the original gains B → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_person_added_to_an_agent_only_dm_starts_a_new_one() {
    let state = state().await;
    let tenant = community(&state, "dm-agent-only-person").await;
    let (a, b, agent) = (Keys::generate(), Keys::generate(), Keys::generate());
    make_agent(&state, &tenant, &agent, &a, "anyone").await;
    let dm = open(&state, &tenant, &a, &[&agent]).await.expect("open");

    let (target, created) = add(&state, &tenant, &a, dm, &[&b], true)
        .await
        .expect("add person");

    assert!(created);
    assert_ne!(target, dm);
    assert_eq!(members(&state, &tenant, dm).await, set_of(&[&a, &agent]));
    assert_eq!(members(&state, &tenant, target).await, set_of(&[&a, &b]));
    assert_eq!(
        add(&state, &tenant, &a, dm, &[&b], true)
            .await
            .expect("add again"),
        (target, false)
    );
    assert_eq!(
        open(&state, &tenant, &a, &[&agent]).await.expect("reopen"),
        dm
    );
}

/// An agent joining a DM still identified by all its members (from before
/// agents were told apart) moves it to its people, so they find it by name.
/// Mutation: compare against the recomputed people hash instead of the stored
/// one → the DM keeps its old fingerprint → A and B get a new DM → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_agent_joining_a_dm_named_by_all_its_members_moves_it_to_its_people() {
    let state = state().await;
    let tenant = community(&state, "dm-legacy-agent-join").await;
    let (a, b, x, y) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    make_agent(&state, &tenant, &x, &a, "anyone").await;
    make_agent(&state, &tenant, &y, &a, "anyone").await;
    // Without the tag, the DM is identified by all its members.
    let dm = open(&state, &tenant, &a, &[&b, &x]).await.expect("open");

    let (target, created) = add(&state, &tenant, &a, dm, &[&y], true)
        .await
        .expect("add agent");

    assert_eq!((target, created), (dm, false));
    assert_eq!(
        open_with_history(&state, &tenant, &b, &[&a])
            .await
            .expect("people"),
        dm
    );
}

/// Reopening a DM with someone else's agent in it is not adding that agent:
/// only agents that join need their add policy.
/// Mutation: check the policy of every named agent → A's reopen is refused →
/// RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn reopening_a_dm_ignores_the_policy_of_agents_already_in_it() {
    let state = state().await;
    let tenant = community(&state, "dm-reopen-owner-only").await;
    let (a, b, agent) = (Keys::generate(), Keys::generate(), Keys::generate());
    make_agent(&state, &tenant, &agent, &b, "owner_only").await;
    let dm = open(&state, &tenant, &a, &[&b]).await.expect("open");
    add(&state, &tenant, &b, dm, &[&agent], true)
        .await
        .expect("owner adds");

    assert_eq!(
        open_with_history(&state, &tenant, &a, &[&b, &agent])
            .await
            .expect("reopen"),
        dm
    );
}

/// An open that loses the race to create a DM still adds the agents it named:
/// the DM it tried to make is committed by another writer while its insert
/// waits, so its insert fails on the identity and it reopens that DM.
/// Mutation: report a lost race as created → the open skips adding X → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_open_that_loses_the_race_still_adds_its_agents() {
    let state = state().await;
    let tenant = community(&state, "dm-open-race").await;
    let (a, b, x) = (Keys::generate(), Keys::generate(), Keys::generate());
    make_agent(&state, &tenant, &x, &a, "anyone").await;
    let (a_pk, b_pk) = (a.public_key().to_bytes(), b.public_key().to_bytes());

    // Make A and B's DM in a transaction left open, so the open's insert of
    // the same identity waits on it.
    let mut rival = state.db.pool().begin().await.expect("begin");
    let rival_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO channels \
             (id, community_id, name, channel_type, visibility, created_by, participant_hash) \
         VALUES ($1, $2, 'DM', 'dm', 'private', $3, $4)",
    )
    .bind(rival_id)
    .bind(tenant.community().as_uuid())
    .bind(a_pk.as_slice())
    .bind(buzz_db::dm::compute_participant_hash(&[a_pk.as_slice(), b_pk.as_slice()]).as_slice())
    .execute(&mut *rival)
    .await
    .expect("rival DM");
    for pk in [&a_pk, &b_pk] {
        sqlx::query(
            "INSERT INTO channel_members (community_id, channel_id, pubkey, role, invited_by) \
             VALUES ($1, $2, $3, 'member', $4)",
        )
        .bind(tenant.community().as_uuid())
        .bind(rival_id)
        .bind(pk.as_slice())
        .bind(a_pk.as_slice())
        .execute(&mut *rival)
        .await
        .expect("rival member");
    }

    let opening = tokio::spawn({
        let (state, tenant, a, b, x) = (
            state.clone(),
            tenant.clone(),
            a.clone(),
            b.clone(),
            x.clone(),
        );
        async move { open_with_history(&state, &tenant, &a, &[&b, &x]).await }
    });
    wait_until_blocked_by(&mut rival).await;
    rival.commit().await.expect("rival commits");

    assert_eq!(opening.await.expect("task").expect("open"), rival_id);
    assert_eq!(
        members(&state, &tenant, rival_id).await,
        set_of(&[&a, &b, &x])
    );
}

/// Wait until another session is waiting on a lock `holder` holds.
async fn wait_until_blocked_by(holder: &mut sqlx::PgConnection) {
    for _ in 0..200 {
        let blocked: i64 = sqlx::query_scalar(
            // pg_locks, unlike pg_stat_activity, is not frozen for the rest
            // of the holder's transaction.
            "SELECT count(*) FROM pg_locks \
             WHERE NOT granted AND pg_backend_pid() = ANY(pg_blocking_pids(pid))",
        )
        .fetch_one(&mut *holder)
        .await
        .expect("blocked sessions");
        if blocked > 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("no session waited on the held lock");
}

/// Reopening a DM identified by all its members with the tag moves it to its
/// people, so the people alone find it afterwards.
/// Mutation: reconcile only when someone joins → A and B get a new DM → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_tagged_reopen_moves_an_old_dm_to_its_people() {
    let state = state().await;
    let tenant = community(&state, "dm-legacy-reopen").await;
    let (a, b, x) = (Keys::generate(), Keys::generate(), Keys::generate());
    make_agent(&state, &tenant, &x, &a, "anyone").await;
    let dm = open(&state, &tenant, &a, &[&b, &x]).await.expect("open");

    assert_eq!(
        open_with_history(&state, &tenant, &a, &[&b, &x])
            .await
            .expect("tagged reopen"),
        dm
    );
    assert_eq!(
        open_with_history(&state, &tenant, &b, &[&a])
            .await
            .expect("people"),
        dm
    );
    assert_eq!(members(&state, &tenant, dm).await, set_of(&[&a, &b, &x]));
}

/// When the people of a DM identified by all its members already have their
/// own DM, an agent added to it joins it in place, and both DMs keep their
/// identities.
/// Mutation: send the agent to the people's DM, or move the fingerprint over
/// it → the agent lands elsewhere or the write fails → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_agent_joins_an_old_dm_in_place_when_its_people_have_another() {
    let state = state().await;
    let tenant = community(&state, "dm-legacy-collision").await;
    let (a, b, x, y) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    make_agent(&state, &tenant, &x, &a, "anyone").await;
    make_agent(&state, &tenant, &y, &a, "anyone").await;
    let peoples = open(&state, &tenant, &a, &[&b]).await.expect("people");
    let old = open(&state, &tenant, &a, &[&b, &x]).await.expect("old");

    assert_eq!(
        add(&state, &tenant, &a, old, &[&y], true)
            .await
            .expect("add agent"),
        (old, false)
    );
    assert_eq!(
        members(&state, &tenant, old).await,
        set_of(&[&a, &b, &x, &y])
    );
    assert_eq!(members(&state, &tenant, peoples).await, set_of(&[&a, &b]));
    assert_eq!(
        open_with_history(&state, &tenant, &b, &[&a])
            .await
            .expect("people"),
        peoples
    );
}

/// A new DM started from a DM carries its people, not the agents in it, so
/// no agent's add policy can block it. Agents named in the add still join.
/// Mutation: carry the source's agents → X, M and Y are in the new DMs, or the
/// policy refuses them → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_new_dm_from_a_dm_carries_only_its_people() {
    let state = state().await;
    let tenant = community(&state, "dm-fork-people").await;
    let (a, b, c, x, m, y) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    make_agent(&state, &tenant, &x, &b, "owner_only").await;
    make_agent(&state, &tenant, &m, &a, "owner_only").await;
    make_agent(&state, &tenant, &y, &b, "nobody").await;

    // A DM of people with someone else's agent: the new DM is its people.
    let trio = open(&state, &tenant, &a, &[&b]).await.expect("open");
    add(&state, &tenant, &b, trio, &[&x], true)
        .await
        .expect("owner adds");
    let (forked, created) = add(&state, &tenant, &a, trio, &[&c], false)
        .await
        .expect("start new DM");
    assert!(created);
    assert_eq!(
        members(&state, &tenant, forked).await,
        set_of(&[&a, &b, &c])
    );
    assert_eq!(members(&state, &tenant, trio).await, set_of(&[&a, &b, &x]));

    // These people now have a DM, but an add without the tag, as older
    // clients send, puts no one into it: it starts a DM of exactly these
    // members.
    let (named, created) = add(&state, &tenant, &a, trio, &[&c, &m], false)
        .await
        .expect("start new DM with an agent");
    assert!(created);
    assert_ne!(named, forked);
    assert_eq!(
        members(&state, &tenant, named).await,
        set_of(&[&a, &b, &c, &m])
    );
    assert_eq!(
        members(&state, &tenant, forked).await,
        set_of(&[&a, &b, &c])
    );
    assert_eq!(members(&state, &tenant, trio).await, set_of(&[&a, &b, &x]));

    // With the tag, the add lands in the DM these people already have, and
    // the agent joins it, with its history.
    let (named, created) = add(&state, &tenant, &a, trio, &[&c, &m], true)
        .await
        .expect("join their DM with an agent");
    assert_eq!((named, created), (forked, false));
    assert_eq!(
        members(&state, &tenant, forked).await,
        set_of(&[&a, &b, &c, &m])
    );

    // A DM of one person and agents, theirs or not: the new DM is people
    // only, and the next one opens it.
    let mut pair = None;
    for (agent, history) in [(&m, true), (&y, false)] {
        let solo = open(&state, &tenant, &a, &[agent]).await.expect("solo");
        let (target, created) = add(&state, &tenant, &a, solo, &[&c], history)
            .await
            .expect("add a person");
        assert_ne!(target, solo);
        assert_eq!(created, pair.is_none());
        assert_eq!(*pair.get_or_insert(target), target);
        assert_eq!(members(&state, &tenant, target).await, set_of(&[&a, &c]));
    }
    let pair = pair.expect("a DM of A and C");

    // From a DM of one person, an agent named with a person: without the
    // tag, a DM of exactly these members; with it, the DM those people
    // already have, which the agent joins with its history.
    let solo = open(&state, &tenant, &a, &[&y]).await.expect("solo");
    let (untagged, created) = add(&state, &tenant, &a, solo, &[&c, &m], false)
        .await
        .expect("add a person and an agent");
    assert!(created);
    assert_ne!(untagged, pair);
    assert_eq!(
        members(&state, &tenant, untagged).await,
        set_of(&[&a, &c, &m])
    );
    assert_eq!(members(&state, &tenant, pair).await, set_of(&[&a, &c]));
    assert_eq!(
        add(&state, &tenant, &a, solo, &[&c, &m], true)
            .await
            .expect("add a person and an agent, with history"),
        (pair, false)
    );
    assert_eq!(members(&state, &tenant, pair).await, set_of(&[&a, &c, &m]));
    assert_eq!(members(&state, &tenant, solo).await, set_of(&[&a, &y]));
}

/// A client from before agents were told apart opens a DM by all its
/// members. A DM whose agents joined it in place holds exactly those members,
/// so the open lands there instead of making another; a DM that holds others
/// too is not theirs.
/// Mutation: drop the people's-DM fallback from untagged opens → a second DM
/// of A, B and X is made → RED; skip its members check → the open of A, B and
/// Y lands in a DM that also holds X → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_untagged_open_finds_the_dm_agents_joined_in_place() {
    let state = state().await;
    let tenant = community(&state, "dm-untagged-grown").await;
    let (a, b, x, y) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    make_agent(&state, &tenant, &x, &a, "owner_only").await;
    make_agent(&state, &tenant, &y, &a, "owner_only").await;
    let dm = open(&state, &tenant, &a, &[&b]).await.expect("open");
    add(&state, &tenant, &a, dm, &[&x], true)
        .await
        .expect("agent joins");

    for (actor, others) in [(&a, [&b, &x]), (&b, [&a, &x])] {
        assert_eq!(
            open(&state, &tenant, actor, &others).await.expect("reopen"),
            dm
        );
    }
    assert_eq!(members(&state, &tenant, dm).await, set_of(&[&a, &b, &x]));

    // Without X, or with another agent, it is not the DM asked for.
    let other = open(&state, &tenant, &a, &[&b, &y]).await.expect("open");
    assert_ne!(other, dm);
    assert_eq!(members(&state, &tenant, other).await, set_of(&[&a, &b, &y]));
}

/// There is one DM per group of people, and agents joining it don't fork it.
/// An open without the tag that names only the people, as an older client
/// sends, opens their DM with the agents in it, whether an agent joined it
/// in place or one of two agents left a DM first made with all of them.
/// Mutation: require exact membership on the first lookup and refuse the
/// people's DM on an insert conflict, giving A and B a separate DM → the
/// open fails → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_untagged_open_of_the_people_finds_their_dm_with_its_agents() {
    let state = state().await;
    let tenant = community(&state, "dm-untagged-people").await;
    let (a, b, c, x, y) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    make_agent(&state, &tenant, &x, &a, "anyone").await;
    make_agent(&state, &tenant, &y, &a, "anyone").await;

    // An agent joins a DM of A and B in place.
    let dm = open(&state, &tenant, &a, &[&b]).await.expect("open");
    add(&state, &tenant, &a, dm, &[&x], true)
        .await
        .expect("agent joins");
    assert_eq!(open(&state, &tenant, &b, &[&a]).await.expect("reopen"), dm);
    assert_eq!(members(&state, &tenant, dm).await, set_of(&[&a, &b, &x]));

    // A DM first made with two agents, one of whom then leaves.
    let group = open(&state, &tenant, &a, &[&c, &x, &y])
        .await
        .expect("open with agents");
    let removed = submit(
        &state,
        &tenant,
        &a,
        9001,
        vec![
            vec!["h".to_string(), group.to_string()],
            vec!["p".into(), hex(&x)],
        ],
    )
    .await
    .expect("remove");
    assert!(removed.accepted, "{}", removed.message);
    assert_eq!(
        open(&state, &tenant, &c, &[&a]).await.expect("reopen"),
        group
    );
    assert_eq!(members(&state, &tenant, group).await, set_of(&[&a, &c, &y]));
}

/// An agent already in a DM and named in an add joins whichever DM the add
/// lands in: a new DM, the DM its people already have, or the new DM a DM of
/// one person starts. It still needs its add policy in a new DM, and the DM
/// the add came from keeps its members.
/// Mutation: leave out additions already in the source → X is missing from
/// each target → RED; exempt them from the policy → Y joins → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_named_agent_from_the_dm_joins_the_dm_the_add_lands_in() {
    let state = state().await;
    let tenant = community(&state, "dm-named-source-agent").await;
    let (a, b, c, d, e, x, y) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    make_agent(&state, &tenant, &x, &b, "anyone").await;
    make_agent(&state, &tenant, &y, &b, "owner_only").await;
    let source = open(&state, &tenant, &a, &[&b]).await.expect("open");
    add(&state, &tenant, &b, source, &[&x, &y], true)
        .await
        .expect("owner adds");
    let source_members = set_of(&[&a, &b, &x, &y]);

    // Start a new DM.
    let (forked, created) = add(&state, &tenant, &a, source, &[&c, &x], false)
        .await
        .expect("start new DM");
    assert!(created);
    assert_eq!(
        members(&state, &tenant, forked).await,
        set_of(&[&a, &b, &c, &x])
    );

    // Include history, landing in the DM these people already have.
    let existing = open(&state, &tenant, &a, &[&b, &d]).await.expect("abd");
    assert_eq!(
        add(&state, &tenant, &a, source, &[&d, &x], true)
            .await
            .expect("include history"),
        (existing, false)
    );
    assert_eq!(
        members(&state, &tenant, existing).await,
        set_of(&[&a, &b, &d, &x])
    );

    // Someone else's owner_only agent named for a new DM is still refused.
    rejected(
        add(&state, &tenant, &a, source, &[&e, &y], false).await,
        "policy:owner_only",
    );
    assert_eq!(members(&state, &tenant, source).await, source_members);

    // A DM of one person starts a new DM, which the named agent joins.
    let solo = open(&state, &tenant, &a, &[&x]).await.expect("solo");
    let (target, created) = add(&state, &tenant, &a, solo, &[&c, &x], true)
        .await
        .expect("add a person");
    assert!(created);
    assert_eq!(
        members(&state, &tenant, target).await,
        set_of(&[&a, &c, &x])
    );
    assert_eq!(members(&state, &tenant, solo).await, set_of(&[&a, &x]));
}

/// A new DM started from a DM holds who is in that DM when it is made, not
/// who was in it when the add arrived.
/// Mutation: build the new DM from members read before taking the DM's lock
/// → C, who left meanwhile, is in it → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_new_dm_from_a_dm_holds_its_current_members() {
    let state = state().await;
    let tenant = community(&state, "dm-fork-live").await;
    let (a, b, c, d) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    let dm = open(&state, &tenant, &a, &[&b, &c]).await.expect("open");

    // Hold the DM's membership lock until the add waits on it, then let C
    // leave and release it. If the add did not take this lock, it would never
    // wait, and the test fails rather than passing.
    let mut lock = state.db.pool().begin().await.expect("begin");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!(
            "buzz_channel_membership:{}:{dm}",
            tenant.community().as_uuid()
        ))
        .execute(&mut *lock)
        .await
        .expect("lock");
    let adding = tokio::spawn({
        let (state, tenant, a, d) = (state.clone(), tenant.clone(), a.clone(), d.clone());
        async move { add(&state, &tenant, &a, dm, &[&d], false).await }
    });
    wait_until_blocked_by(&mut lock).await;
    sqlx::query(
        "UPDATE channel_members SET removed_at = NOW() \
         WHERE channel_id = $1 AND pubkey = $2",
    )
    .bind(dm)
    .bind(c.public_key().to_bytes().as_slice())
    .execute(&mut *lock)
    .await
    .expect("C leaves");
    lock.commit().await.expect("release");

    let (target, created) = adding.await.expect("task").expect("add");
    assert!(created);
    assert_eq!(
        members(&state, &tenant, target).await,
        set_of(&[&a, &b, &d])
    );
}

/// The relay joins no DM but its moderation DM.
/// Mutation: drop the relay-key check on additions → the relay joins → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn the_relay_cannot_be_added_to_a_dm() {
    let state = state().await;
    let tenant = community(&state, "dm-add-relay").await;
    let (a, b) = (Keys::generate(), Keys::generate());
    let dm = open(&state, &tenant, &a, &[&b]).await.expect("open");

    for history in [true, false] {
        rejected(
            add(&state, &tenant, &a, dm, &[&state.relay_keypair], history).await,
            "cannot be changed",
        );
    }
    assert_eq!(members(&state, &tenant, dm).await, set_of(&[&a, &b]));
}

/// The limit counts the caller only if they are a person: an agent may open a
/// DM with nine people, and no more.
/// Mutation: count the caller as a person → the agent's DM is refused → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_agent_can_open_a_dm_with_nine_people() {
    let state = state().await;
    let tenant = community(&state, "dm-agent-nine").await;
    let (owner, agent) = (Keys::generate(), Keys::generate());
    make_agent(&state, &tenant, &agent, &owner, "anyone").await;
    let people: Vec<Keys> = (0..9).map(|_| Keys::generate()).collect();
    let nine: Vec<&Keys> = people.iter().collect();

    let dm = open(&state, &tenant, &agent, &nine)
        .await
        .expect("nine people");
    let mut everyone = nine.clone();
    everyone.push(&agent);
    assert_eq!(members(&state, &tenant, dm).await, set_of(&everyone));

    let mut ten = nine;
    ten.push(&owner);
    rejected(
        open(&state, &tenant, &agent, &ten).await,
        "at most 9 people",
    );
}

/// Removing an agent from a DM identified by all its members (from before
/// agents were told apart) leaves it its people's DM, whether its owner removes
/// it or it leaves, unless they already have another DM.
/// Mutation: give the identity up on every removal → B gets a new DM → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn removing_an_agent_moves_an_old_dm_to_its_people() {
    let state = state().await;
    let tenant = community(&state, "dm-legacy-remove-agent").await;
    let (a, b, c, d, x) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    make_agent(&state, &tenant, &x, &a, "anyone").await;
    let removed = open(&state, &tenant, &a, &[&b, &x]).await.expect("open");
    let left = open(&state, &tenant, &a, &[&c, &x]).await.expect("open");
    let peoples = open(&state, &tenant, &a, &[&d]).await.expect("people");
    let shadowed = open(&state, &tenant, &a, &[&d, &x]).await.expect("open");

    let remove = |dm: Uuid| {
        vec![
            vec!["h".to_string(), dm.to_string()],
            vec!["p".into(), hex(&x)],
        ]
    };
    for dm in [removed, shadowed] {
        let result = submit(&state, &tenant, &a, 9001, remove(dm))
            .await
            .expect("remove");
        assert!(result.accepted, "{}", result.message);
    }
    let result = submit(
        &state,
        &tenant,
        &x,
        9022,
        vec![vec!["h".into(), left.to_string()]],
    )
    .await
    .expect("leave");
    assert!(result.accepted, "{}", result.message);

    for (person, dm) in [(&b, removed), (&c, left), (&d, peoples)] {
        assert_eq!(
            open_with_history(&state, &tenant, person, &[&a])
                .await
                .expect("reopen"),
            dm
        );
    }
    assert_ne!(
        open(&state, &tenant, &a, &[&d, &x]).await.expect("reopen"),
        shadowed,
        "a DM that could not take its people's identity gives its own up"
    );
}

/// A tagged add that lands in the DM its people already have checks agents
/// against that DM: one already in it needs no permission.
/// Mutation: check policies before finding the other DM → the add is refused
/// → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn agents_are_checked_against_the_dm_they_join() {
    let state = state().await;
    let tenant = community(&state, "dm-existing-policy").await;
    let (a, b, c, x) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    make_agent(&state, &tenant, &x, &b, "owner_only").await;
    let trio = open(&state, &tenant, &a, &[&b, &c]).await.expect("trio");
    add(&state, &tenant, &b, trio, &[&x], true)
        .await
        .expect("owner adds");
    let pair = open(&state, &tenant, &a, &[&b]).await.expect("pair");

    assert_eq!(
        add(&state, &tenant, &a, pair, &[&c, &x], true)
            .await
            .expect("add"),
        (trio, false)
    );
    assert_eq!(members(&state, &tenant, pair).await, set_of(&[&a, &b]));
}

/// A DM someone left keeps their history, so an agent joining it, even as the
/// only other member, needs its add policy.
/// Mutation: exempt any DM of the actor and one agent → the agent joins → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_agent_needs_its_policy_to_join_a_dm_someone_left() {
    let state = state().await;
    let tenant = community(&state, "dm-left-policy").await;
    let (a, b, x) = (Keys::generate(), Keys::generate(), Keys::generate());
    make_agent(&state, &tenant, &x, &b, "owner_only").await;
    let dm = open(&state, &tenant, &a, &[&b]).await.expect("open");
    let result = submit(
        &state,
        &tenant,
        &b,
        9022,
        vec![vec!["h".into(), dm.to_string()]],
    )
    .await
    .expect("leave");
    assert!(result.accepted, "{}", result.message);

    rejected(
        add(&state, &tenant, &a, dm, &[&x], true).await,
        "policy:owner_only",
    );
    assert_eq!(members(&state, &tenant, dm).await, set_of(&[&a]));
}

/// A new DM starts only from a live, unarchived DM, checked under its lock.
/// Mutation: skip the check when starting a new DM → the archived DM forks →
/// RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_new_dm_cannot_start_from_an_archived_dm() {
    let state = state().await;
    let tenant = community(&state, "dm-fork-archived").await;
    let (a, b, c) = (Keys::generate(), Keys::generate(), Keys::generate());
    let dm = open(&state, &tenant, &a, &[&b]).await.expect("open");
    sqlx::query("UPDATE channels SET archived_at = NOW() WHERE id = $1")
        .bind(dm)
        .execute(state.db.pool())
        .await
        .expect("archive");

    let c_pk = c.public_key().to_bytes();
    let result = state
        .db
        .fork_dm(
            tenant.community(),
            dm,
            &a.public_key().to_bytes(),
            &[c_pk.as_slice()],
            true,
            &|_: &str, _: &[u8]| Ok(()),
        )
        .await;
    assert!(
        matches!(&result, Err(buzz_db::DbError::AccessDenied(m)) if m.contains("archived")),
        "{result:?}"
    );
}

/// Adds `others` to `dm` tagged `["history", "ask"]`.
async fn ask(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    actor: &Keys,
    dm: Uuid,
    others: &[&Keys],
) -> Result<(Uuid, bool), IngestError> {
    let mut tags: Vec<Vec<String>> = vec![vec!["h".into(), dm.to_string()]];
    tags.extend(others.iter().map(|k| vec!["p".into(), hex(k)]));
    tags.push(vec!["history".into(), "ask".into()]);
    submit(state, tenant, actor, 41011, tags)
        .await
        .map(|r| response_channel(&r))
}

/// Asking first, agents alone join in place with the DM's history, as with
/// `include`.
/// Mutation: refuse every ask → the agent is not added → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn asking_adds_agents_alone_in_place() {
    let state = state().await;
    let tenant = community(&state, "dm-ask-agents").await;
    let (a, b, agent) = (Keys::generate(), Keys::generate(), Keys::generate());
    make_agent(&state, &tenant, &agent, &a, "anyone").await;
    let dm = open(&state, &tenant, &a, &[&b]).await.expect("open");

    let (target, created) = ask(&state, &tenant, &a, dm, &[&agent]).await.expect("ask");

    assert_eq!((target, created), (dm, false));
    assert_eq!(
        members(&state, &tenant, dm).await,
        set_of(&[&a, &b, &agent])
    );
}

/// Asking first, a person joining a DM of two or more people is refused with
/// the exact reason, nothing changes, and the refused command is not
/// remembered, so the same event is refused again rather than taken as done.
/// Agents named with the person do not join either. Mutations: grow as with
/// `include` → the person joins → RED; commit the command before refusing →
/// the second send is a duplicate → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn asking_refuses_people_joining_a_dm_of_two_people() {
    let state = state().await;
    let tenant = community(&state, "dm-ask-people").await;
    let (a, b, c, agent) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    make_agent(&state, &tenant, &agent, &a, "anyone").await;
    let dm = open(&state, &tenant, &a, &[&b]).await.expect("open");

    let event = EventBuilder::new(Kind::Custom(41011), "")
        .tags(
            [
                vec!["h".to_string(), dm.to_string()],
                vec!["p".into(), hex(&c)],
                vec!["p".into(), hex(&agent)],
                vec!["history".into(), "ask".into()],
            ]
            .into_iter()
            .map(|tag| Tag::parse(tag).expect("tag")),
        )
        .sign_with_keys(&a)
        .expect("sign");
    for _ in 0..2 {
        match ingest_event(&state, &tenant, event.clone(), http_auth(&a)).await {
            Err(IngestError::Rejected(message)) => assert_eq!(message, "needs-choice: history"),
            Err(other) => panic!("expected the history choice, got {other:?}"),
            Ok(result) => panic!("expected the history choice, got {:?}", result.message),
        }
    }

    assert_eq!(members(&state, &tenant, dm).await, set_of(&[&a, &b]));
    assert_eq!(open(&state, &tenant, &a, &[&b]).await.expect("reopen"), dm);
    let (target, created) = add(&state, &tenant, &a, dm, &[&c, &agent], true)
        .await
        .expect("add with history");
    assert_eq!((target, created), (dm, false));
    assert_eq!(
        members(&state, &tenant, dm).await,
        set_of(&[&a, &b, &c, &agent])
    );
}

/// Asking first, a person added to a DM of one person and their agents starts
/// a new DM without a choice, as with `include`, and the DM stays theirs.
/// Mutation: check for a choice before the one-person rule → refused → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn asking_starts_a_new_dm_for_a_person_added_to_a_dm_of_one_person() {
    let state = state().await;
    let tenant = community(&state, "dm-ask-one-person").await;
    let (a, c, agent) = (Keys::generate(), Keys::generate(), Keys::generate());
    make_agent(&state, &tenant, &agent, &a, "anyone").await;
    let dm = open(&state, &tenant, &a, &[&agent]).await.expect("open");

    let (target, created) = ask(&state, &tenant, &a, dm, &[&c]).await.expect("ask");

    assert!(created);
    assert_ne!(target, dm);
    assert_eq!(members(&state, &tenant, dm).await, set_of(&[&a, &agent]));
    assert_eq!(members(&state, &tenant, target).await, set_of(&[&a, &c]));
}

/// An agent cannot ask either, and only adds take `ask`.
/// Mutations: let agents ask → the agent's add is answered → RED; accept
/// `ask` on kind:41010 → the open succeeds → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn only_people_ask_and_only_when_adding() {
    let state = state().await;
    let tenant = community(&state, "dm-ask-who").await;
    let (a, b, c, agent) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    make_agent(&state, &tenant, &agent, &a, "anyone").await;
    let dm = open(&state, &tenant, &a, &[&b]).await.expect("open");
    add(&state, &tenant, &a, dm, &[&agent], true)
        .await
        .expect("add agent");

    rejected(ask(&state, &tenant, &agent, dm, &[&c]).await, "only people");
    let tags = vec![
        vec!["p".into(), hex(&b)],
        vec!["history".into(), "ask".into()],
    ];
    rejected(
        submit(&state, &tenant, &a, 41010, tags).await.map(|_| ()),
        "history",
    );
}
