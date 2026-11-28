//! Postgres-backed tests for relay-provisioned system channels: the relay
//! creates and signs the channel, other keys can't claim its label, and only
//! an artifact's author may change it there.

use std::sync::Arc;

use buzz_core::TenantContext;
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use uuid::Uuid;

use super::group_state_postgres_tests::{
    create_channel, edit_channel, group_state_tags, http_auth,
};
use crate::handlers::ingest::ingest_event;
use crate::state::AppState;

const LABEL: &str = "example-records";

async fn fixture() -> (Arc<AppState>, TenantContext) {
    let mut state = crate::api::bridge::postgres_tests::bridge_handler_test_state()
        .await
        .expect("local Postgres and Redis");
    Arc::make_mut(
        &mut Arc::get_mut(&mut state)
            .expect("test state has a single owner")
            .config,
    )
    .provisioned_system_channels = vec![LABEL.to_string()];
    let host = format!("provisioned-{}.local", Uuid::new_v4().simple());
    let community = state
        .db
        .ensure_configured_community(&host)
        .await
        .expect("ensure community")
        .id;
    (state, TenantContext::resolved(community, &host))
}

async fn provisioned(state: &Arc<AppState>, tenant: &TenantContext) -> Uuid {
    super::provisioned_channels::ensure_for_community(state, tenant)
        .await
        .expect("ensure provisioned channels");
    state
        .db
        .ensure_provisioned_system_channel(
            tenant.community(),
            LABEL,
            &state.relay_keypair.public_key().to_bytes(),
        )
        .await
        .expect("provisioned channel")
        .channel
        .id
}

async fn ingest(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    keys: &Keys,
    event: Event,
) -> Result<(), String> {
    match ingest_event(state, tenant, event, http_auth(keys)).await {
        Ok(result) if result.accepted => Ok(()),
        Ok(result) => Err(result.message),
        Err(error) => Err(format!("{error:?}")),
    }
}

fn sign(keys: &Keys, kind: u16, tags: Vec<Vec<String>>) -> Event {
    EventBuilder::new(Kind::Custom(kind), "")
        .tags(tags.into_iter().map(|t| Tag::parse(t).unwrap()))
        .sign_with_keys(keys)
        .expect("sign")
}

fn revision(keys: &Keys, d: Uuid, op: &str, home: Uuid, prev: Option<&Event>) -> Event {
    let mut tags = vec![
        vec!["ar".into(), "1".into()],
        vec!["d".into(), d.to_string()],
        vec!["h".into(), home.to_string()],
        vec!["type".into(), "example.record".into()],
        vec!["op".into(), op.into()],
    ];
    if op != "delete" {
        tags.push(vec!["title".into(), "Record".into()]);
    }
    if let Some(prev) = prev {
        tags.push(vec!["prev".into(), prev.id.to_hex()]);
    }
    sign(keys, 45010, tags)
}

async fn join(state: &Arc<AppState>, tenant: &TenantContext, keys: &Keys, channel: Uuid) {
    ingest(
        state,
        tenant,
        keys,
        sign(keys, 9021, vec![vec!["h".into(), channel.to_string()]]),
    )
    .await
    .expect("join");
}

/// The relay creates one open system channel per label and signs its
/// kind:39000 with the system type, the label and itself as creator.
/// Mutation: skip emit_group_discovery_events on create → RED.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn relay_creates_and_signs_the_provisioned_channel() {
    let (state, tenant) = fixture().await;
    let channel = provisioned(&state, &tenant).await;
    let again = provisioned(&state, &tenant).await;
    assert_eq!(channel, again);
    let tags = group_state_tags(&state, &tenant, channel, 39000).await;
    let relay = state.relay_keypair.public_key().to_hex();
    for expected in [
        vec!["t".to_string(), "system".to_string()],
        vec!["t".to_string(), LABEL.to_string()],
        vec!["P".to_string(), relay],
    ] {
        assert!(tags.contains(&expected), "missing {expected:?} in {tags:?}");
    }
}

/// A member can't create or relabel a channel with a provisioned label, and
/// other labels still work. Mutation: drop either reserved_label check → RED.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn members_cannot_claim_a_provisioned_label() {
    let (state, tenant) = fixture().await;
    let member = Keys::generate();
    let squat = Uuid::new_v4();
    let error = ingest(
        &state,
        &tenant,
        &member,
        sign(
            &member,
            9007,
            vec![
                vec!["h".into(), squat.to_string()],
                vec!["name".into(), "decoy".into()],
                vec!["channel_type".into(), "system".into()],
                vec!["visibility".into(), "open".into()],
                vec!["t".into(), LABEL.into()],
            ],
        ),
    )
    .await
    .expect_err("a member's channel with the label");
    assert!(error.contains("reserved for the relay"), "{error}");

    let own = create_channel(
        &state,
        &tenant,
        &member,
        &[
            &["channel_type", "system"],
            &["visibility", "open"],
            &["t", "example-other"],
        ],
    )
    .await;
    let error = edit_channel(&state, &tenant, &member, own, &[&["t", LABEL]])
        .await
        .expect_err("relabel to a provisioned label");
    assert!(error.contains("reserved for the relay"), "{error}");
    edit_channel(&state, &tenant, &member, own, &[&["t", "example-third"]])
        .await
        .expect("an ordinary label");
}

/// In the provisioned channel another member can't revise or delete an
/// author's record, the author can, and an ordinary channel still lets any
/// writer with the head revise it. Mutation: pass no labels to
/// accept_artifact → RED.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn only_the_author_changes_records_in_the_provisioned_channel() {
    let (state, tenant) = fixture().await;
    let channel = provisioned(&state, &tenant).await;
    let author = Keys::generate();
    let other = Keys::generate();
    join(&state, &tenant, &author, channel).await;
    join(&state, &tenant, &other, channel).await;

    let d = Uuid::new_v4();
    let create = revision(&author, d, "create", channel, None);
    ingest(&state, &tenant, &author, create.clone())
        .await
        .expect("create");
    for op in ["update", "delete"] {
        let error = ingest(
            &state,
            &tenant,
            &other,
            revision(&other, d, op, channel, Some(&create)),
        )
        .await
        .expect_err("another member's change");
        assert!(
            error.contains("only the artifact's author"),
            "{op}: {error}"
        );
    }
    let update = revision(&author, d, "update", channel, Some(&create));
    ingest(&state, &tenant, &author, update.clone())
        .await
        .expect("author update");
    ingest(
        &state,
        &tenant,
        &author,
        revision(&author, d, "delete", channel, Some(&update)),
    )
    .await
    .expect("author delete");

    let ordinary = create_channel(&state, &tenant, &author, &[&["visibility", "open"]]).await;
    join(&state, &tenant, &other, ordinary).await;
    let d = Uuid::new_v4();
    let create = revision(&author, d, "create", ordinary, None);
    ingest(&state, &tenant, &author, create.clone())
        .await
        .expect("ordinary create");
    ingest(
        &state,
        &tenant,
        &other,
        revision(&other, d, "update", ordinary, Some(&create)),
    )
    .await
    .expect("an ordinary channel keeps shared editing");
}
