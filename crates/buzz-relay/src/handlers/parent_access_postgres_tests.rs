//! Postgres-backed tests for reading and writing a private channel through
//! its parents (`buzz_db::channel_tree`): every read and write goes through
//! the real ingest, REQ and COUNT paths, and no access ever makes a member
//! row.

use std::sync::Arc;

use buzz_core::TenantContext;
use nostr::{EventBuilder, Keys, Kind, Tag};
use uuid::Uuid;

use crate::handlers::group_state_postgres_tests::{
    community, create_channel, edit_channel, group_state_tags, http_auth, state, test_conn,
};
use crate::handlers::ingest::ingest_event;
use crate::state::AppState;

/// Ingest a kind:9000 that `actor` signs, adding `person` to `channel` with
/// `role`.
pub(crate) async fn add(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    actor: &Keys,
    channel: Uuid,
    person: &Keys,
    role: &str,
) {
    let event = EventBuilder::new(Kind::Custom(9000), "")
        .tags([
            Tag::parse(["h", &channel.to_string()]).unwrap(),
            Tag::parse(["p", &person.public_key().to_hex()]).unwrap(),
            Tag::parse(["role", role]).unwrap(),
        ])
        .sign_with_keys(actor)
        .unwrap();
    let result = ingest_event(state, tenant, event, http_auth(actor))
        .await
        .expect("add member");
    assert!(result.accepted, "add member: {:?}", result.message);
}

/// Ingest a kind:9001 that `actor` signs, removing `person` from `channel`.
async fn remove(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    actor: &Keys,
    channel: Uuid,
    person: &Keys,
) {
    let event = EventBuilder::new(Kind::Custom(9001), "")
        .tags([
            Tag::parse(["h", &channel.to_string()]).unwrap(),
            Tag::parse(["p", &person.public_key().to_hex()]).unwrap(),
        ])
        .sign_with_keys(actor)
        .unwrap();
    let result = ingest_event(state, tenant, event, http_auth(actor))
        .await
        .expect("remove member");
    assert!(result.accepted, "remove member: {:?}", result.message);
}

/// Post a kind:9 that `author` signs in `channel`; the event ID when the
/// relay accepted it, its message otherwise.
pub(crate) async fn post(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    author: &Keys,
    channel: Uuid,
) -> Result<String, String> {
    let event = EventBuilder::new(Kind::Custom(9), format!("hello {}", Uuid::new_v4()))
        .tags([Tag::parse(["h", &channel.to_string()]).unwrap()])
        .sign_with_keys(author)
        .unwrap();
    let id = event.id.to_hex();
    match ingest_event(state, tenant, event, http_auth(author)).await {
        Ok(result) if result.accepted => Ok(id),
        Ok(result) => Err(result.message),
        Err(error) => Err(format!("{error:?}")),
    }
}

/// Set `child`'s parent to `parent`, or clear it, as `actor`.
pub(crate) async fn set_parent(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    actor: &Keys,
    child: Uuid,
    parent: Option<Uuid>,
) -> Result<(), String> {
    let value = parent.map(|p| p.to_string()).unwrap_or_default();
    edit_channel(state, tenant, actor, child, &[&["parent", &value]]).await
}

/// The IDs of the events `reader` gets for one historical REQ before EOSE;
/// none when the relay closes it.
pub(crate) async fn req_event_ids(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    reader: &Keys,
    filter: serde_json::Value,
) -> Vec<String> {
    let (conn, mut send_rx) = test_conn(tenant, reader);
    let filter: nostr::Filter = serde_json::from_value(filter).expect("filter");
    crate::handlers::req::handle_req("g".into(), vec![filter], vec![None], conn, state.clone())
        .await;
    let mut ids = Vec::new();
    while let Ok(frame) = send_rx.try_recv() {
        let axum::extract::ws::Message::Text(text) = frame else {
            continue;
        };
        let message: serde_json::Value = serde_json::from_str(&text).expect("relay frame");
        match message[0].as_str() {
            Some("EVENT") => ids.push(message[2]["id"].as_str().expect("id").to_owned()),
            Some("EOSE") => return ids,
            Some("CLOSED") => {
                assert!(ids.is_empty(), "closed after events: {text}");
                return ids;
            }
            other => panic!("unexpected frame {other:?}: {text}"),
        }
    }
    panic!("no EOSE; events so far: {ids:?}");
}

/// The kind:9 event IDs `reader` gets for a REQ naming `channel`.
pub(crate) async fn read_named(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    reader: &Keys,
    channel: Uuid,
) -> Vec<String> {
    req_event_ids(
        state,
        tenant,
        reader,
        serde_json::json!({"kinds": [9], "#h": [channel.to_string()]}),
    )
    .await
}

/// The kind:9 event IDs `reader` gets for a REQ that names no channel.
pub(crate) async fn read_implicit(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    reader: &Keys,
) -> Vec<String> {
    req_event_ids(state, tenant, reader, serde_json::json!({"kinds": [9]})).await
}

/// The COUNT `reader` gets for kind:9 in `channel`, or `None` when refused.
pub(crate) async fn count_named(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    reader: &Keys,
    channel: Uuid,
) -> Option<u64> {
    let (conn, mut send_rx) = test_conn(tenant, reader);
    let filter: nostr::Filter =
        serde_json::from_value(serde_json::json!({"kinds": [9], "#h": [channel.to_string()]}))
            .unwrap();
    crate::handlers::count::handle_count("c".into(), vec![filter], conn, state.clone()).await;
    while let Ok(frame) = send_rx.try_recv() {
        let axum::extract::ws::Message::Text(text) = frame else {
            continue;
        };
        let message: serde_json::Value = serde_json::from_str(&text).unwrap();
        match message[0].as_str() {
            Some("COUNT") => return message[2]["count"].as_u64(),
            Some("CLOSED") => return None,
            _ => {}
        }
    }
    panic!("no COUNT reply");
}

pub(crate) async fn role_in(
    state: &AppState,
    tenant: &TenantContext,
    channel: Uuid,
    person: &Keys,
) -> Option<String> {
    state
        .db
        .get_member_role(tenant.community(), channel, &person.public_key().to_bytes())
        .await
        .expect("member role")
}

/// A private project, a private child under it, and an outsider.
pub(crate) struct Tree {
    pub(crate) state: Arc<AppState>,
    pub(crate) tenant: TenantContext,
    pub(crate) owner: Keys,
    pub(crate) reader: Keys,
    pub(crate) outsider: Keys,
    pub(crate) project: Uuid,
    pub(crate) child: Uuid,
}

impl Tree {
    /// `reader` is a member of the project only.
    pub(crate) async fn new(label: &str) -> Self {
        Self::with_project(label, &[&["visibility", "private"]]).await
    }

    /// `project_edit`, when not empty, is applied to the project with
    /// kind:9002 after it is created.
    async fn with_project(label: &str, project_edit: &[&[&str]]) -> Self {
        let state = state().await;
        let tenant = community(&state, label).await;
        let owner = Keys::generate();
        let reader = Keys::generate();
        let outsider = Keys::generate();
        let project = create_channel(&state, &tenant, &owner, &[]).await;
        if !project_edit.is_empty() {
            edit_channel(&state, &tenant, &owner, project, project_edit)
                .await
                .expect("edit project");
        }
        let child = create_channel(&state, &tenant, &owner, &[&["visibility", "private"]]).await;
        set_parent(&state, &tenant, &owner, child, Some(project))
            .await
            .expect("set parent");
        add(&state, &tenant, &owner, project, &reader, "member").await;
        Self {
            state,
            tenant,
            owner,
            reader,
            outsider,
            project,
            child,
        }
    }
}

/// A member of a private project reads, counts and posts in a private child
/// with no row there; an outsider does none of it, by name or implicitly,
/// and no access makes a member row.
/// Mutation: drop the descendant union from `get_member_channel_ids`, or the
/// parent arm from `get_readable_named_only_channels`, or the parent path
/// from `check_channel_membership` → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_project_member_reads_and_posts_in_a_private_child() {
    let t = Tree::new("parent-read").await;
    let (state, tenant) = (&t.state, &t.tenant);
    let message = post(state, tenant, &t.owner, t.child)
        .await
        .expect("owner posts");

    assert_eq!(
        read_named(state, tenant, &t.reader, t.child).await,
        vec![message.clone()]
    );
    assert!(read_implicit(state, tenant, &t.reader)
        .await
        .contains(&message));
    assert_eq!(
        count_named(state, tenant, &t.reader, t.child).await,
        Some(1)
    );
    let reply = post(state, tenant, &t.reader, t.child)
        .await
        .expect("reader posts");

    assert!(read_named(state, tenant, &t.outsider, t.child)
        .await
        .is_empty());
    let implicit = read_implicit(state, tenant, &t.outsider).await;
    assert!(!implicit.contains(&message) && !implicit.contains(&reply));
    assert!(matches!(
        count_named(state, tenant, &t.outsider, t.child).await,
        None | Some(0)
    ));
    assert!(post(state, tenant, &t.outsider, t.child).await.is_err());

    assert_eq!(role_in(state, tenant, t.child, &t.reader).await, None);
    let members = state
        .db
        .get_members(tenant.community(), t.child)
        .await
        .expect("members");
    assert_eq!(members.len(), 1, "only the owner has a row: {members:?}");
}

/// Removal from the project revokes the child at once, with every cache
/// warmed first: no stale answer survives the removal.
/// Mutation: skip the per-person invalidation on removal, or cache
/// inherited answers per channel → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn removal_from_the_project_revokes_the_child_at_once() {
    let t = Tree::new("parent-revoke").await;
    let (state, tenant) = (&t.state, &t.tenant);
    let message = post(state, tenant, &t.owner, t.child).await.unwrap();
    assert_eq!(
        read_named(state, tenant, &t.reader, t.child).await,
        vec![message.clone()]
    );
    assert!(read_implicit(state, tenant, &t.reader)
        .await
        .contains(&message));

    remove(state, tenant, &t.owner, t.project, &t.reader).await;

    assert!(read_named(state, tenant, &t.reader, t.child)
        .await
        .is_empty());
    assert!(!read_implicit(state, tenant, &t.reader)
        .await
        .contains(&message));
    assert!(post(state, tenant, &t.reader, t.child).await.is_err());
}

/// Making an open project private revokes the children its openness let
/// everyone read.
/// Mutation: drop the accessible-channels invalidation on a visibility
/// flip → RED (the outsider's cached set still holds the child).
#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_open_project_made_private_revokes_its_children() {
    let t = Tree::with_project("parent-flip", &[]).await;
    let (state, tenant) = (&t.state, &t.tenant);
    let message = post(state, tenant, &t.owner, t.child).await.unwrap();
    assert_eq!(
        read_named(state, tenant, &t.outsider, t.child).await,
        vec![message.clone()]
    );
    assert!(read_implicit(state, tenant, &t.outsider)
        .await
        .contains(&message));

    edit_channel(
        state,
        tenant,
        &t.owner,
        t.project,
        &[&["visibility", "private"]],
    )
    .await
    .expect("make private");

    assert!(read_named(state, tenant, &t.outsider, t.child)
        .await
        .is_empty());
    assert!(!read_implicit(state, tenant, &t.outsider)
        .await
        .contains(&message));
    assert_eq!(
        read_named(state, tenant, &t.reader, t.child).await,
        vec![message]
    );
}

/// Moving a child out from under a project revokes the project's readers,
/// and clearing a parent does the same.
/// Mutation: drop `invalidate_all_accessible_channels` on a parent change
/// → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn re_parenting_revokes_the_old_parents_readers() {
    let t = Tree::new("parent-move").await;
    let (state, tenant) = (&t.state, &t.tenant);
    let other = create_channel(state, tenant, &t.owner, &[&["visibility", "private"]]).await;
    let message = post(state, tenant, &t.owner, t.child).await.unwrap();
    assert!(read_implicit(state, tenant, &t.reader)
        .await
        .contains(&message));

    set_parent(state, tenant, &t.owner, t.child, Some(other))
        .await
        .expect("move");
    assert!(read_named(state, tenant, &t.reader, t.child)
        .await
        .is_empty());
    assert!(!read_implicit(state, tenant, &t.reader)
        .await
        .contains(&message));

    add(state, tenant, &t.owner, other, &t.reader, "member").await;
    assert_eq!(
        read_named(state, tenant, &t.reader, t.child).await,
        vec![message.clone()]
    );
    set_parent(state, tenant, &t.owner, t.child, None)
        .await
        .expect("clear");
    assert!(read_named(state, tenant, &t.reader, t.child)
        .await
        .is_empty());
}

/// Loops, a fourth level, and a parent under a channel with children too
/// deep are refused; a channel cannot be its own parent.
/// Mutation: drop the loop or depth check in `decide_parent_change` → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn loops_and_deep_nesting_are_refused() {
    let state = state().await;
    let tenant = community(&state, "parent-depth").await;
    let owner = Keys::generate();
    let private: &[&[&str]] = &[&["visibility", "private"]];
    let a = create_channel(&state, &tenant, &owner, private).await;
    let b = create_channel(&state, &tenant, &owner, private).await;
    let c = create_channel(&state, &tenant, &owner, private).await;
    let d = create_channel(&state, &tenant, &owner, private).await;
    let e = create_channel(&state, &tenant, &owner, private).await;
    set_parent(&state, &tenant, &owner, b, Some(a))
        .await
        .expect("b under a");
    set_parent(&state, &tenant, &owner, c, Some(b))
        .await
        .expect("c under b");

    assert!(
        set_parent(&state, &tenant, &owner, a, Some(c))
            .await
            .is_err(),
        "loop"
    );
    assert!(
        set_parent(&state, &tenant, &owner, a, Some(b))
            .await
            .is_err(),
        "loop"
    );
    assert!(
        set_parent(&state, &tenant, &owner, a, Some(a))
            .await
            .is_err(),
        "self"
    );
    assert!(
        set_parent(&state, &tenant, &owner, d, Some(c))
            .await
            .is_err(),
        "4 deep"
    );
    // a has two levels under it, so it cannot go under anything.
    assert!(
        set_parent(&state, &tenant, &owner, a, Some(e))
            .await
            .is_err(),
        "4 deep"
    );
    // d with a child cannot go under b, which already has a parent.
    set_parent(&state, &tenant, &owner, e, Some(d))
        .await
        .expect("e under d");
    assert!(
        set_parent(&state, &tenant, &owner, d, Some(b))
            .await
            .is_err(),
        "4 deep"
    );
    set_parent(&state, &tenant, &owner, d, Some(a))
        .await
        .expect("d under a: 3 deep");
}

/// Roles through a parent cap at member, and setting a parent takes a direct
/// owner or admin row in the child; a project admin may still clear it.
/// Mutation: let an inherited admin keep admin, or let a parent's admin set
/// a parent → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn inherited_admin_is_capped_but_may_clear_the_parent() {
    let t = Tree::new("parent-admin").await;
    let (state, tenant) = (&t.state, &t.tenant);
    let admin = Keys::generate();
    add(state, tenant, &t.owner, t.project, &admin, "admin").await;
    let elsewhere = create_channel(state, tenant, &admin, &[&["visibility", "private"]]).await;

    post(state, tenant, &admin, t.child)
        .await
        .expect("admin posts as member");
    assert!(
        edit_channel(state, tenant, &admin, t.child, &[&["about", "x"]])
            .await
            .is_err()
    );
    assert!(set_parent(state, tenant, &admin, t.child, Some(elsewhere))
        .await
        .is_err());
    // A plain member of the project may not clear it.
    assert!(set_parent(state, tenant, &t.reader, t.child, None)
        .await
        .is_err());

    set_parent(state, tenant, &admin, t.child, None)
        .await
        .expect("project admin clears");
    assert!(read_named(state, tenant, &admin, t.child).await.is_empty());
    assert!(post(state, tenant, &admin, t.child).await.is_err());
}

/// Setting a parent takes a role that may post in the new parent: the
/// child's owner cannot hang it under a channel they cannot write in.
/// Mutation: drop the `can_post` check on the new parent → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn setting_a_parent_needs_write_access_to_it() {
    let t = Tree::new("parent-write").await;
    let (state, tenant) = (&t.state, &t.tenant);
    let theirs = create_channel(state, tenant, &t.outsider, &[&["visibility", "private"]]).await;
    let announce = create_channel(state, tenant, &t.outsider, &[]).await;
    edit_channel(
        state,
        tenant,
        &t.outsider,
        announce,
        &[&["join_role", "guest"]],
    )
    .await
    .expect("announce");

    assert!(set_parent(state, tenant, &t.owner, t.child, Some(theirs))
        .await
        .is_err());
    assert!(set_parent(state, tenant, &t.owner, t.child, Some(announce))
        .await
        .is_err());
    add(state, tenant, &t.outsider, announce, &t.owner, "member").await;
    set_parent(state, tenant, &t.owner, t.child, Some(announce))
        .await
        .expect("a member of the announce channel may");
}

/// The closest grant wins: under an open announce project, a non-member of
/// the project reads the child but may not post, as a guest; a member of
/// the project posts.
/// Mutation: let any open ancestor grant member → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_announce_project_grants_its_children_as_guest() {
    let t = Tree::with_project("parent-announce", &[&["join_role", "guest"]]).await;
    let (state, tenant) = (&t.state, &t.tenant);
    let message = post(state, tenant, &t.owner, t.child).await.unwrap();

    assert_eq!(
        read_named(state, tenant, &t.outsider, t.child).await,
        vec![message]
    );
    assert!(post(state, tenant, &t.outsider, t.child).await.is_err());
    post(state, tenant, &t.reader, t.child)
        .await
        .expect("project member posts");
}

/// Children of an unlisted open project are readable by name only.
/// Mutation: let unlisted open channels seed the implicit scope → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_unlisted_projects_children_are_named_only() {
    let t = Tree::with_project("parent-unlisted", &[&["unlisted", "true"]]).await;
    let (state, tenant) = (&t.state, &t.tenant);
    let message = post(state, tenant, &t.owner, t.child).await.unwrap();

    assert_eq!(
        read_named(state, tenant, &t.outsider, t.child).await,
        vec![message.clone()]
    );
    assert!(!read_implicit(state, tenant, &t.outsider)
        .await
        .contains(&message));
}

/// kind:39000 names the parent, and the child's metadata is readable by the
/// project's readers.
/// Mutation: drop the `parent` tag from `group_metadata_tags` → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn metadata_names_the_parent() {
    let t = Tree::new("parent-metadata").await;
    let (state, tenant) = (&t.state, &t.tenant);
    let tags = group_state_tags(state, tenant, t.child, 39000).await;
    assert!(
        tags.contains(&vec!["parent".to_string(), t.project.to_string()]),
        "{tags:?}"
    );
    let metadata = req_event_ids(
        state,
        tenant,
        &t.reader,
        serde_json::json!({"kinds": [39000], "#d": [t.child.to_string()]}),
    )
    .await;
    assert_eq!(
        metadata.len(),
        1,
        "a project reader reads the child's metadata"
    );

    // A parent tag rides alone.
    assert!(edit_channel(
        state,
        tenant,
        &t.owner,
        t.child,
        &[&["parent", ""], &["about", "x"]]
    )
    .await
    .is_err());
}

/// An attachment posted in a private child loads for a project reader and
/// not for an outsider.
/// Mutation: drop the parent arm from `media_reference_query` → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_project_reader_loads_a_childs_attachments() {
    let t = Tree::new("parent-media").await;
    let (state, tenant) = (&t.state, &t.tenant);
    let hash = "c".repeat(64);
    let event = EventBuilder::new(Kind::Custom(9), "media")
        .tags([
            Tag::parse(["h", &t.child.to_string()]).unwrap(),
            Tag::parse([
                "imeta",
                &format!("url /media/{hash}.bin"),
                &format!("x {hash}"),
            ])
            .unwrap(),
        ])
        .sign_with_keys(&t.owner)
        .unwrap();
    // Stored directly: the attachment's upload metadata is not under test.
    state
        .db
        .insert_event(tenant.community(), &event, Some(t.child))
        .await
        .expect("store attachment");
    let base = format!("https://{}/media", tenant.host());
    let access = |keys: &Keys| {
        let pubkey = keys.public_key().to_bytes();
        let (state, base, hash) = (state.clone(), base.clone(), hash.clone());
        async move {
            state
                .db
                .media_access(
                    tenant.community(),
                    &hash,
                    "bin",
                    &base,
                    &pubkey,
                    false,
                    |_, _| true,
                )
                .await
                .expect("media access")
        }
    };
    assert_eq!(
        access(&t.reader).await,
        buzz_db::media::MediaAccess::Allowed
    );
    assert_eq!(
        access(&t.outsider).await,
        buzz_db::media::MediaAccess::Restricted
    );
}

/// An archived project still lets its readers read a private child.
/// Mutation: filter archived ancestors out of `get_channel_ancestors` → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_archived_project_still_grants_its_children() {
    let t = Tree::new("parent-archived").await;
    let (state, tenant) = (&t.state, &t.tenant);
    let message = post(state, tenant, &t.owner, t.child).await.unwrap();

    edit_channel(state, tenant, &t.owner, t.project, &[&["archived", "true"]])
        .await
        .expect("archive project");

    assert_eq!(
        read_named(state, tenant, &t.reader, t.child).await,
        vec![message]
    );
    assert!(read_named(state, tenant, &t.outsider, t.child)
        .await
        .is_empty());
}

/// Set `channel`'s `parent_access` to `value` as `actor`.
pub(crate) async fn set_parent_access(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    actor: &Keys,
    channel: Uuid,
    value: &str,
) -> Result<(), String> {
    edit_channel(state, tenant, actor, channel, &[&["parent_access", value]]).await
}

/// The kind:39000 events `reader` gets for a REQ naming `channel`.
async fn metadata_named(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    reader: &Keys,
    channel: Uuid,
) -> usize {
    req_event_ids(
        state,
        tenant,
        reader,
        serde_json::json!({"kinds": [39000], "#d": [channel.to_string()]}),
    )
    .await
    .len()
}

/// A "Project can view" child: the project reads it, by name and implicitly,
/// but cannot post; a direct member row still posts; metadata says so.
/// Mutation: drop the guest cap from `closest_grant` → RED (the reader
/// posts).
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_view_only_child_lets_the_project_read_but_not_post() {
    let t = Tree::new("parent-access-guest").await;
    let (state, tenant) = (&t.state, &t.tenant);
    set_parent_access(state, tenant, &t.owner, t.child, "guest")
        .await
        .expect("narrow to guest");
    let message = post(state, tenant, &t.owner, t.child).await.unwrap();

    assert_eq!(
        read_named(state, tenant, &t.reader, t.child).await,
        vec![message.clone()]
    );
    assert!(read_implicit(state, tenant, &t.reader)
        .await
        .contains(&message));
    assert!(post(state, tenant, &t.reader, t.child).await.is_err());

    let direct = Keys::generate();
    add(state, tenant, &t.owner, t.project, &direct, "member").await;
    add(state, tenant, &t.owner, t.child, &direct, "member").await;
    post(state, tenant, &direct, t.child)
        .await
        .expect("a direct member posts");

    let tags = group_state_tags(state, tenant, t.child, 39000).await;
    assert!(
        tags.contains(&vec!["parent_access".to_string(), "guest".to_string()]),
        "{tags:?}"
    );
    assert_eq!(role_in(state, tenant, t.child, &t.reader).await, None);

    // Back to the default: the project posts again, and the tag is gone.
    set_parent_access(state, tenant, &t.owner, t.child, "member")
        .await
        .expect("widen to member");
    post(state, tenant, &t.reader, t.child)
        .await
        .expect("the project posts again");
    let tags = group_state_tags(state, tenant, t.child, 39000).await;
    assert!(
        !tags.iter().any(|tag| tag[0] == "parent_access"),
        "{tags:?}"
    );
}

/// An "Only people you add" child is hidden from the project: no named or
/// implicit read, no count, no metadata, no post, with every cache warmed
/// first. Direct rows still win, including for someone outside the project.
/// Mutation: drop the `c.parent_access` gate from `through_parent_sql`, or
/// the gate from `descendants_sql` → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_restricted_child_is_hidden_from_the_project() {
    let t = Tree::new("parent-access-none").await;
    let (state, tenant) = (&t.state, &t.tenant);
    let message = post(state, tenant, &t.owner, t.child).await.unwrap();
    assert!(read_implicit(state, tenant, &t.reader)
        .await
        .contains(&message));
    assert_eq!(metadata_named(state, tenant, &t.reader, t.child).await, 1);

    set_parent_access(state, tenant, &t.owner, t.child, "none")
        .await
        .expect("restrict");

    assert!(read_named(state, tenant, &t.reader, t.child)
        .await
        .is_empty());
    assert!(!read_implicit(state, tenant, &t.reader)
        .await
        .contains(&message));
    assert!(matches!(
        count_named(state, tenant, &t.reader, t.child).await,
        None | Some(0)
    ));
    assert_eq!(metadata_named(state, tenant, &t.reader, t.child).await, 0);
    assert!(post(state, tenant, &t.reader, t.child).await.is_err());

    // A direct row, as guest, reads; an outsider added directly posts.
    add(state, tenant, &t.owner, t.child, &t.reader, "guest").await;
    assert_eq!(
        read_named(state, tenant, &t.reader, t.child).await,
        vec![message.clone()]
    );
    assert!(post(state, tenant, &t.reader, t.child).await.is_err());
    add(state, tenant, &t.owner, t.child, &t.outsider, "member").await;
    post(state, tenant, &t.outsider, t.child)
        .await
        .expect("a directly added outsider posts");
    assert!(read_named(state, tenant, &t.outsider, t.project)
        .await
        .is_empty());
}

/// A restricted channel cuts its children off from the project: a task
/// conversation under it is readable by the channel's own members, not by
/// the project's.
/// Mutation: drop the `p.parent_access` gate on the grandparent arm of
/// `through_parent_sql`, or the middle gate from `descendants_sql`, or stop
/// `Lineage::reachable` at the child only → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_restricted_channel_cuts_off_its_task_conversations() {
    let t = Tree::new("parent-access-grandchild").await;
    let (state, tenant) = (&t.state, &t.tenant);
    let task = create_channel(state, tenant, &t.owner, &[&["visibility", "private"]]).await;
    set_parent(state, tenant, &t.owner, task, Some(t.child))
        .await
        .expect("file the task");
    let channel_member = Keys::generate();
    add(state, tenant, &t.owner, t.child, &channel_member, "member").await;
    let message = post(state, tenant, &t.owner, task).await.unwrap();
    assert_eq!(
        read_named(state, tenant, &t.reader, task).await,
        vec![message.clone()]
    );

    set_parent_access(state, tenant, &t.owner, t.child, "none")
        .await
        .expect("restrict the channel");

    assert!(read_named(state, tenant, &t.reader, task).await.is_empty());
    assert!(!read_implicit(state, tenant, &t.reader)
        .await
        .contains(&message));
    assert!(post(state, tenant, &t.reader, task).await.is_err());
    assert_eq!(
        read_named(state, tenant, &channel_member, task).await,
        vec![message.clone()]
    );
    assert!(read_implicit(state, tenant, &channel_member)
        .await
        .contains(&message));
    post(state, tenant, &channel_member, task)
        .await
        .expect("the channel's member posts in its task");

    // A view-only channel caps its task's project readers at guest too.
    set_parent_access(state, tenant, &t.owner, t.child, "guest")
        .await
        .expect("narrow to guest");
    assert_eq!(read_named(state, tenant, &t.reader, task).await.len(), 2);
    assert!(post(state, tenant, &t.reader, task).await.is_err());
    post(state, tenant, &channel_member, task)
        .await
        .expect("the channel's own member is not capped");
}

/// Only the channel's own owner or admin changes `parent_access`: not the
/// project's admin, not a channel member. Bad values and mixing it with
/// other metadata are refused; it may ride with `parent`.
/// Mutation: skip `decide_parent_access_change` → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn only_the_channels_own_admin_changes_parent_access() {
    let t = Tree::new("parent-access-authz").await;
    let (state, tenant) = (&t.state, &t.tenant);
    let project_admin = Keys::generate();
    add(state, tenant, &t.owner, t.project, &project_admin, "admin").await;
    let channel_member = Keys::generate();
    add(state, tenant, &t.owner, t.child, &channel_member, "member").await;
    let channel_admin = Keys::generate();
    add(state, tenant, &t.owner, t.child, &channel_admin, "admin").await;

    for actor in [&project_admin, &channel_member, &t.reader] {
        assert!(
            set_parent_access(state, tenant, actor, t.child, "none")
                .await
                .is_err(),
            "a non-admin of the channel may not restrict it"
        );
    }
    for bad in ["", "admin", "None"] {
        assert!(set_parent_access(state, tenant, &t.owner, t.child, bad)
            .await
            .is_err());
    }
    assert!(edit_channel(
        state,
        tenant,
        &t.owner,
        t.child,
        &[&["parent_access", "none"], &["about", "x"]]
    )
    .await
    .is_err());
    assert!(edit_channel(
        state,
        tenant,
        &t.owner,
        t.child,
        &[&["parent_access", "none"], &["parent_access", "guest"]]
    )
    .await
    .is_err());
    // Nothing changed so far.
    post(state, tenant, &t.reader, t.child)
        .await
        .expect("still open to the project");

    set_parent_access(state, tenant, &channel_admin, t.child, "none")
        .await
        .expect("the channel's admin restricts it");
    assert!(read_named(state, tenant, &t.reader, t.child)
        .await
        .is_empty());

    // Filed and narrowed in one event.
    let chat = create_channel(state, tenant, &t.owner, &[&["visibility", "private"]]).await;
    edit_channel(
        state,
        tenant,
        &t.owner,
        chat,
        &[
            &["parent", &t.project.to_string()],
            &["parent_access", "guest"],
        ],
    )
    .await
    .expect("file and narrow together");
    let note = post(state, tenant, &t.owner, chat).await.unwrap();
    assert_eq!(read_named(state, tenant, &t.reader, chat).await, vec![note]);
    assert!(post(state, tenant, &t.reader, chat).await.is_err());
}

/// Expire `channel`'s TTL now and run the reaper, which must archive it.
pub(crate) async fn auto_archive(state: &Arc<AppState>, tenant: &TenantContext, channel: Uuid) {
    sqlx::query(
        "UPDATE channels SET ttl_deadline = clock_timestamp() - interval '1 second' \
         WHERE community_id = $1 AND id = $2",
    )
    .bind(tenant.community().as_uuid())
    .bind(channel)
    .execute(state.db.pool())
    .await
    .expect("expire ttl");
    let reaped = state
        .db
        .reap_expired_ephemeral_channels()
        .await
        .expect("reap");
    assert!(
        reaped.iter().any(|r| r.channel_id == channel),
        "the reaper archives the expired channel: {reaped:?}"
    );
}

/// A chat filed in a project may auto-archive. The project's readers post
/// in it before, and find and read it after, by name, implicitly, by COUNT
/// and through its metadata; an outsider still gets nothing.
/// Mutation: refuse a TTL on any channel with a parent → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_filed_chat_auto_archives_and_stays_readable_through_its_project() {
    let t = Tree::new("parent-auto-archive").await;
    let (state, tenant) = (&t.state, &t.tenant);
    edit_channel(state, tenant, &t.owner, t.child, &[&["ttl", "3600"]])
        .await
        .expect("a filed chat takes a TTL");
    let first = post(state, tenant, &t.owner, t.child).await.unwrap();
    let second = post(state, tenant, &t.reader, t.child)
        .await
        .expect("a project reader posts in the auto-archiving chat");

    auto_archive(state, tenant, t.child).await;

    let mut read = read_named(state, tenant, &t.reader, t.child).await;
    read.sort();
    let mut expected = vec![first.clone(), second.clone()];
    expected.sort();
    assert_eq!(read, expected);
    let implicit = read_implicit(state, tenant, &t.reader).await;
    assert!(implicit.contains(&first) && implicit.contains(&second));
    assert_eq!(
        count_named(state, tenant, &t.reader, t.child).await,
        Some(2)
    );
    assert_eq!(metadata_named(state, tenant, &t.reader, t.child).await, 1);
    let tags = group_state_tags(state, tenant, t.child, 39000).await;
    assert!(
        tags.contains(&vec!["parent".to_string(), t.project.to_string()]),
        "{tags:?}"
    );
    assert!(read_named(state, tenant, &t.outsider, t.child)
        .await
        .is_empty());
    assert!(!read_implicit(state, tenant, &t.outsider)
        .await
        .contains(&first));
    assert_eq!(role_in(state, tenant, t.child, &t.reader).await, None);
}

/// A project, a channel with children and no parent, never auto-archives:
/// not by taking a TTL, not by having a channel filed under it while it has
/// one, and not by losing its own parent while it has one and children. An
/// auto-archiving chat in a project may still hold task conversations.
/// Mutation: set `PROJECTS_MAY_AUTO_ARCHIVE` → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_project_never_auto_archives() {
    let t = Tree::new("parent-project-ttl").await;
    let (state, tenant) = (&t.state, &t.tenant);
    assert!(
        edit_channel(state, tenant, &t.owner, t.project, &[&["ttl", "3600"]])
            .await
            .is_err(),
        "a project takes no TTL"
    );

    edit_channel(state, tenant, &t.owner, t.child, &[&["ttl", "3600"]])
        .await
        .expect("a filed chat takes a TTL");
    let task = create_channel(state, tenant, &t.owner, &[&["visibility", "private"]]).await;
    set_parent(state, tenant, &t.owner, task, Some(t.child))
        .await
        .expect("an auto-archiving filed chat holds a task conversation");
    assert!(
        set_parent(state, tenant, &t.owner, t.child, None)
            .await
            .is_err(),
        "clearing its parent would leave an auto-archiving project"
    );

    let standalone = create_channel(state, tenant, &t.owner, &[]).await;
    edit_channel(state, tenant, &t.owner, standalone, &[&["ttl", "3600"]])
        .await
        .expect("a channel outside any tree takes a TTL");
    let other = create_channel(state, tenant, &t.owner, &[]).await;
    assert!(
        set_parent(state, tenant, &t.owner, other, Some(standalone))
            .await
            .is_err(),
        "filing under it would make it an auto-archiving project"
    );
    set_parent(state, tenant, &t.owner, standalone, Some(t.project))
        .await
        .expect("an auto-archiving channel is filed in a project");

    // Without children, a filed auto-archiving channel may leave its project.
    set_parent(state, tenant, &t.owner, standalone, None)
        .await
        .expect("a childless auto-archiving channel leaves its project");
}
