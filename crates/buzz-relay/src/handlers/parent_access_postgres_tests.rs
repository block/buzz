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
async fn add(
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
async fn post(
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
async fn set_parent(
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
async fn req_event_ids(
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
async fn read_named(
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
async fn read_implicit(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    reader: &Keys,
) -> Vec<String> {
    req_event_ids(state, tenant, reader, serde_json::json!({"kinds": [9]})).await
}

/// The COUNT `reader` gets for kind:9 in `channel`, or `None` when refused.
async fn count_named(
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

async fn role_in(
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
struct Tree {
    state: Arc<AppState>,
    tenant: TenantContext,
    owner: Keys,
    reader: Keys,
    outsider: Keys,
    project: Uuid,
    child: Uuid,
}

impl Tree {
    /// `reader` is a member of the project only.
    async fn new(label: &str) -> Self {
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
/// project's readers. A channel in a tree cannot take a TTL.
/// Mutation: drop the `parent` tag from `group_metadata_tags`, or the TTL
/// refusal → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn metadata_names_the_parent_and_a_tree_has_no_ttl() {
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

    for channel in [t.child, t.project] {
        assert!(
            edit_channel(state, tenant, &t.owner, channel, &[&["ttl", "3600"]])
                .await
                .is_err(),
            "a channel in a tree takes no TTL"
        );
    }
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
