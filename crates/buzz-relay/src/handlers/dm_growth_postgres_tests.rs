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

/// Only people share a DM's history: an agent's add with the tag is refused,
/// and without it starts a new DM, as before.
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

/// A DM of one person and their agents never gains a person in place, even
/// with the history tag: adding one starts a DM of everyone, and adding them
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
    assert_eq!(
        members(&state, &tenant, target).await,
        set_of(&[&a, &b, &agent])
    );
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
