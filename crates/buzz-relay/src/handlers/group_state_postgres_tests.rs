//! Postgres-backed tests for the relay-signed NIP-29 group-state events
//! (kinds 39000–39002) that channel creation and edits publish.

use std::sync::Arc;

use buzz_core::TenantContext;
use buzz_db::event::EventQuery;
use nostr::{EventBuilder, Keys, Kind, Tag};
use uuid::Uuid;

use crate::handlers::ingest::{ingest_event, HttpAuthMethod, IngestAuth};
use crate::state::AppState;
use buzz_auth::Scope;

pub(crate) async fn state() -> Arc<AppState> {
    crate::state::tests::test_state_with_database_url(&crate::test_support::database_url()).await
}

pub(crate) async fn community(state: &AppState, label: &str) -> TenantContext {
    let host = format!("{label}-{}.test", Uuid::new_v4().simple());
    let community = state
        .db
        .ensure_configured_community(&host)
        .await
        .expect("ensure community")
        .id;
    TenantContext::resolved(community, &host)
}

pub(crate) fn http_auth(keys: &Keys) -> IngestAuth {
    IngestAuth::Http {
        pubkey: keys.public_key(),
        scopes: vec![
            Scope::ChannelsWrite,
            Scope::MessagesWrite,
            Scope::AdminChannels,
            Scope::ChannelsRead,
        ],
        auth_method: HttpAuthMethod::Nip98,
    }
}

/// Ingest a kind:9007 that `keys` signs, with `extra` tags after the ID and
/// name, and return the new channel's ID.
pub(crate) async fn create_channel(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    keys: &Keys,
    extra: &[&[&str]],
) -> Uuid {
    let channel_id = Uuid::new_v4();
    let mut tags = vec![
        Tag::parse(["h", &channel_id.to_string()]).unwrap(),
        Tag::parse(["name", &format!("group-state-{}", channel_id.simple())]).unwrap(),
    ];
    for tag in extra {
        tags.push(Tag::parse(tag.iter().copied()).unwrap());
    }
    let event = EventBuilder::new(Kind::Custom(9007), "")
        .tags(tags)
        .sign_with_keys(keys)
        .expect("sign 9007");
    ingest_event(state, tenant, event, http_auth(keys))
        .await
        .unwrap_or_else(|error| panic!("create channel: {error:?}"));
    channel_id
}

/// The current relay-signed group-state event of `kind` for `channel_id`.
pub(crate) async fn group_state_tags(
    state: &AppState,
    tenant: &TenantContext,
    channel_id: Uuid,
    kind: i32,
) -> Vec<Vec<String>> {
    let events = state
        .db
        .query_events(&EventQuery {
            kinds: Some(vec![kind]),
            d_tag: Some(channel_id.to_string()),
            limit: Some(1),
            ..EventQuery::for_community(tenant.community())
        })
        .await
        .expect("query group state");
    let event = events
        .first()
        .unwrap_or_else(|| panic!("no kind:{kind} for {channel_id}"));
    event
        .event
        .tags
        .iter()
        .map(|tag| tag.as_slice().to_vec())
        .collect()
}

/// kind:39000 carries the channel's creation time, and an edit keeps it.
/// Mutation: use the edit time instead of `channels.created_at` → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn metadata_carries_the_channel_creation_time() {
    let state = state().await;
    let tenant = community(&state, "group-state-created-at").await;
    let creator = Keys::generate();
    let channel_id = create_channel(&state, &tenant, &creator, &[]).await;

    let created_at = |tags: &[Vec<String>]| -> Vec<String> {
        tags.iter()
            .filter(|tag| tag[0] == "created_at")
            .map(|tag| tag[1].clone())
            .collect()
    };
    let stored = state
        .db
        .get_channel_for_event_write(tenant.community(), channel_id)
        .await
        .expect("channel")
        .created_at
        .timestamp();
    let tags = group_state_tags(&state, &tenant, channel_id, 39000).await;
    assert_eq!(created_at(&tags), vec![stored.to_string()], "{tags:?}");

    // Move the creation time far into the past, then edit. The new 39000
    // must still report the creation time, not the time of the edit.
    sqlx::query("UPDATE channels SET created_at = to_timestamp(1577836800) WHERE id = $1")
        .bind(channel_id)
        .execute(state.db.pool())
        .await
        .expect("backdate channel");
    let edit = EventBuilder::new(Kind::Custom(9002), "")
        .tags([
            Tag::parse(["h", &channel_id.to_string()]).unwrap(),
            Tag::parse(["name", &format!("renamed-{}", channel_id.simple())]).unwrap(),
        ])
        .sign_with_keys(&creator)
        .unwrap();
    ingest_event(&state, &tenant, edit, http_auth(&creator))
        .await
        .expect("edit channel");
    let tags = group_state_tags(&state, &tenant, channel_id, 39000).await;
    assert!(
        tags.iter()
            .any(|tag| tag[0] == "name" && tag[1].starts_with("renamed-")),
        "edit published: {tags:?}"
    );
    assert_eq!(
        created_at(&tags),
        vec!["1577836800".to_string()],
        "{tags:?}"
    );
}

/// Every group-state kind carries the channel type and the creator, so
/// `#t` and `#P` filters match 39001 and 39002 as well as 39000.
/// Mutation: drop the identity tags from 39001 or 39002 → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn group_state_events_carry_type_then_creator() {
    let state = state().await;
    let tenant = community(&state, "group-state-identity").await;
    let creator = Keys::generate();
    let channel_id = create_channel(
        &state,
        &tenant,
        &creator,
        &[&["channel_type", "forum"], &["visibility", "private"]],
    )
    .await;

    let creator_hex = creator.public_key().to_hex();
    for kind in [39000, 39001, 39002] {
        let tags = group_state_tags(&state, &tenant, channel_id, kind).await;
        let t_tags: Vec<&Vec<String>> = tags.iter().filter(|tag| tag[0] == "t").collect();
        assert_eq!(
            t_tags.first().map(|tag| tag[1].as_str()),
            Some("forum"),
            "kind:{kind} must lead with the channel type: {tags:?}"
        );
        let creators: Vec<&str> = tags
            .iter()
            .filter(|tag| tag[0] == "P")
            .map(|tag| tag[1].as_str())
            .collect();
        assert_eq!(
            creators,
            vec![creator_hex.as_str()],
            "kind:{kind}: {tags:?}"
        );
    }
}

/// The creator tag names the kind:9007 signer, not the current owner.
/// Mutation: derive `P` from the owner role → RED after the transfer.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn creator_tag_survives_ownership_transfer() {
    let state = state().await;
    let tenant = community(&state, "group-state-creator").await;
    let creator = Keys::generate();
    let successor = Keys::generate();
    let channel_id = create_channel(&state, &tenant, &creator, &[]).await;

    // Add the successor as owner, then the creator leaves.
    let channel_hex = channel_id.to_string();
    let add = EventBuilder::new(Kind::Custom(9000), "")
        .tags([
            Tag::parse(["h", &channel_hex]).unwrap(),
            Tag::parse(["p", &successor.public_key().to_hex()]).unwrap(),
            Tag::parse(["role", "owner"]).unwrap(),
        ])
        .sign_with_keys(&creator)
        .unwrap();
    ingest_event(&state, &tenant, add, http_auth(&creator))
        .await
        .expect("add successor");
    let leave = EventBuilder::new(Kind::Custom(9022), "")
        .tags([Tag::parse(["h", &channel_hex]).unwrap()])
        .sign_with_keys(&creator)
        .unwrap();
    ingest_event(&state, &tenant, leave, http_auth(&creator))
        .await
        .expect("creator leaves");

    let roster = group_state_tags(&state, &tenant, channel_id, 39002).await;
    assert!(
        !roster
            .iter()
            .any(|tag| tag[0] == "p" && tag[1] == creator.public_key().to_hex()),
        "creator left the roster: {roster:?}"
    );
    let creator_hex = creator.public_key().to_hex();
    for kind in [39001, 39002] {
        let tags = group_state_tags(&state, &tenant, channel_id, kind).await;
        assert!(
            tags.iter()
                .any(|tag| tag[0] == "P" && tag[1] == creator_hex),
            "kind:{kind} keeps the creator: {tags:?}"
        );
    }
}

/// Ingest a kind:9002 that `keys` signs for `channel_id` with `tags`.
async fn edit_channel(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    keys: &Keys,
    channel_id: Uuid,
    tags: &[&[&str]],
) -> Result<(), String> {
    let mut built = vec![Tag::parse(["h", &channel_id.to_string()]).unwrap()];
    for tag in tags {
        built.push(Tag::parse(tag.iter().copied()).unwrap());
    }
    let event = EventBuilder::new(Kind::Custom(9002), "")
        .tags(built)
        .sign_with_keys(keys)
        .expect("sign 9002");
    match ingest_event(state, tenant, event, http_auth(keys)).await {
        Ok(result) if result.accepted => Ok(()),
        Ok(result) => Err(result.message),
        Err(error) => Err(format!("{error:?}")),
    }
}

/// The `t` values of a group-state event, in tag order.
async fn t_values(
    state: &AppState,
    tenant: &TenantContext,
    channel_id: Uuid,
    kind: i32,
) -> Vec<String> {
    group_state_tags(state, tenant, channel_id, kind)
        .await
        .into_iter()
        .filter(|tag| tag[0] == "t")
        .map(|tag| tag[1].clone())
        .collect()
}

/// Labels from kind:9007 follow the type tag on every group-state kind,
/// in the order given, without duplicates, and the creator `P` stays.
/// Mutation: publish labels before the type, or drop them from 39002 → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn create_publishes_labels_after_the_type() {
    let state = state().await;
    let tenant = community(&state, "group-state-labels").await;
    let creator = Keys::generate();
    let channel_id = create_channel(
        &state,
        &tenant,
        &creator,
        &[
            &["channel_type", "forum"],
            &["t", "workspace"],
            &["t", "team:core"],
            &["t", "workspace"],
        ],
    )
    .await;

    let record = state
        .db
        .get_channel(tenant.community(), channel_id)
        .await
        .expect("channel");
    assert_eq!(record.labels, vec!["workspace", "team:core"]);
    for kind in [39000, 39001, 39002] {
        assert_eq!(
            t_values(&state, &tenant, channel_id, kind).await,
            vec!["forum", "workspace", "team:core"],
            "kind:{kind}"
        );
        let tags = group_state_tags(&state, &tenant, channel_id, kind).await;
        assert!(
            tags.iter()
                .any(|tag| tag[0] == "P" && tag[1] == creator.public_key().to_hex()),
            "kind:{kind} keeps the creator: {tags:?}"
        );
    }
}

/// kind:9002 `t` tags replace the whole set, a lone `["t", ""]` clears
/// it, and an edit without `t` tags leaves it alone.
/// Mutation: append instead of replace, or treat `""` as a label → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn edit_replaces_and_clears_labels() {
    let state = state().await;
    let tenant = community(&state, "group-state-label-edit").await;
    let owner = Keys::generate();
    let channel_id = create_channel(&state, &tenant, &owner, &[&["t", "old"]]).await;

    edit_channel(
        &state,
        &tenant,
        &owner,
        channel_id,
        &[&["t", "new-a"], &["t", "new-b"]],
    )
    .await
    .expect("replace labels");
    for kind in [39000, 39001, 39002] {
        assert_eq!(
            t_values(&state, &tenant, channel_id, kind).await,
            vec!["stream", "new-a", "new-b"],
            "kind:{kind} after replace"
        );
    }

    edit_channel(&state, &tenant, &owner, channel_id, &[&["about", "x"]])
        .await
        .expect("edit about");
    assert_eq!(
        t_values(&state, &tenant, channel_id, 39000).await,
        vec!["stream", "new-a", "new-b"],
        "an edit without t tags keeps the labels"
    );

    edit_channel(&state, &tenant, &owner, channel_id, &[&["t", ""]])
        .await
        .expect("clear labels");
    for kind in [39000, 39001, 39002] {
        assert_eq!(
            t_values(&state, &tenant, channel_id, kind).await,
            vec!["stream"],
            "kind:{kind} after clear"
        );
    }
}

/// Bad label sets are refused before storage, on create and on edit.
/// Mutation: skip validation in either path → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn invalid_labels_are_refused() {
    let state = state().await;
    let tenant = community(&state, "group-state-label-invalid").await;
    let owner = Keys::generate();

    for bad in [vec!["t", "Upper"], vec!["t", "forum"], vec!["t"]] {
        let channel_id = Uuid::new_v4();
        let event = EventBuilder::new(Kind::Custom(9007), "")
            .tags([
                Tag::parse(["h", &channel_id.to_string()]).unwrap(),
                Tag::parse(["name", "bad-labels"]).unwrap(),
                Tag::parse(bad.clone()).unwrap(),
            ])
            .sign_with_keys(&owner)
            .unwrap();
        let accepted = ingest_event(&state, &tenant, event, http_auth(&owner))
            .await
            .is_ok_and(|result| result.accepted);
        assert!(!accepted, "create with {bad:?} must fail");
        assert!(
            state
                .db
                .get_channel(tenant.community(), channel_id)
                .await
                .is_err(),
            "no channel row for {bad:?}"
        );
    }

    let channel_id = create_channel(&state, &tenant, &owner, &[&["t", "keep"]]).await;
    let nine: Vec<String> = (0..9).map(|i| format!("l{i}")).collect();
    let too_many: Vec<[&str; 2]> = nine.iter().map(|l| ["t", l.as_str()]).collect();
    let too_many: Vec<&[&str]> = too_many.iter().map(|t| t.as_slice()).collect();
    for tags in [
        vec![&["t", "workflow"][..]],
        vec![&["t", ""][..], &["t", "x"][..]],
        too_many,
    ] {
        assert!(
            edit_channel(&state, &tenant, &owner, channel_id, &tags)
                .await
                .is_err(),
            "edit with {tags:?} must fail"
        );
    }
    assert_eq!(
        t_values(&state, &tenant, channel_id, 39000).await,
        vec!["stream", "keep"]
    );
}

/// Only an owner or admin may change labels; a plain member may not.
/// Mutation: drop `t` from the privileged tag list → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn members_cannot_change_labels() {
    let state = state().await;
    let tenant = community(&state, "group-state-label-auth").await;
    let owner = Keys::generate();
    let member = Keys::generate();
    let channel_id = create_channel(&state, &tenant, &owner, &[&["t", "keep"]]).await;
    let join = EventBuilder::new(Kind::Custom(9021), "")
        .tags([Tag::parse(["h", &channel_id.to_string()]).unwrap()])
        .sign_with_keys(&member)
        .unwrap();
    ingest_event(&state, &tenant, join, http_auth(&member))
        .await
        .expect("member joins");

    assert!(
        edit_channel(&state, &tenant, &member, channel_id, &[&["t", "mine"]])
            .await
            .is_err(),
        "a member must not set labels"
    );
    assert_eq!(
        t_values(&state, &tenant, channel_id, 39000).await,
        vec!["stream", "keep"]
    );
}
