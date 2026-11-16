//! Postgres-backed tests for artifact conversations (NOSTR.md, Artifact
//! conversations): a task's comments live in a channel at an address derived
//! from the task, filed under the task's home, with no members of its own.
//! Every step goes through the real ingest, REQ and COUNT paths.

use std::sync::Arc;

use buzz_core::TenantContext;
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use uuid::Uuid;

use buzz_auth::Scope;

use crate::handlers::group_state_postgres_tests::{create_channel, edit_channel, test_conn};
use crate::handlers::ingest::{ingest_event, HttpAuthMethod, IngestAuth};
use crate::handlers::parent_access_postgres_tests::{
    add, auto_archive, count_named, post, read_implicit, read_named, role_in, set_parent,
    set_parent_access, Tree,
};
use crate::state::AppState;

/// Ingest `event` as its signer; `Err` carries the refusal.
pub(crate) async fn send(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    event: Event,
) -> Result<(), String> {
    let auth = IngestAuth::Http {
        pubkey: event.pubkey,
        scopes: vec![
            Scope::ChannelsWrite,
            Scope::MessagesWrite,
            Scope::AdminChannels,
            Scope::ChannelsRead,
        ],
        auth_method: HttpAuthMethod::Nip98,
    };
    match ingest_event(state, tenant, event, auth).await {
        Ok(result) if result.accepted => Ok(()),
        Ok(result) => Err(result.message),
        Err(error) => Err(format!("{error:?}")),
    }
}

/// A `buzz.task` revision of `id` in `home`, signed by `key`.
pub(crate) fn task(key: &Keys, home: Uuid, id: Uuid, op: &str, prev: Option<&Event>) -> Event {
    let mut tags = vec![
        vec!["ar".to_owned(), "1".into()],
        vec!["d".into(), id.to_string()],
        vec!["h".into(), home.to_string()],
        vec!["type".into(), "buzz.task".into()],
        vec!["op".into(), op.into()],
    ];
    if op != "delete" {
        tags.push(vec!["title".into(), "Review".into()]);
    }
    if let Some(prev) = prev {
        tags.push(vec!["prev".into(), prev.id.to_hex()]);
    }
    EventBuilder::new(Kind::Custom(45010), if op == "delete" { "" } else { "x" })
        .tags(tags.into_iter().map(|t| Tag::parse(t).unwrap()))
        .sign_with_keys(key)
        .unwrap()
}

/// Create a task in `home` as `key`: its ID and create revision.
pub(crate) async fn create_task(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    key: &Keys,
    home: Uuid,
) -> (Uuid, Event) {
    let id = Uuid::new_v4();
    let event = task(key, home, id, "create", None);
    send(state, tenant, event.clone())
        .await
        .expect("create task");
    (id, event)
}

/// A kind:9007 opening `artifact`'s conversation at `h`.
fn open_event(key: &Keys, artifact: &str, h: Uuid) -> Event {
    EventBuilder::new(Kind::Custom(9007), "")
        .tags([
            Tag::parse(["h", &h.to_string()]).unwrap(),
            Tag::parse(["artifact", artifact]).unwrap(),
        ])
        .sign_with_keys(key)
        .unwrap()
}

/// Open `artifact`'s conversation as `key`, at its derived address.
pub(crate) async fn open(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    key: &Keys,
    artifact: Uuid,
) -> Result<Uuid, String> {
    let conversation = buzz_core::artifact::conversation_id(&artifact.to_string());
    send(
        state,
        tenant,
        open_event(key, &artifact.to_string(), conversation),
    )
    .await
    .map(|()| conversation)
}

/// A kind:9005 by `actor` deleting `target` in `channel`.
async fn moderate(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    actor: &Keys,
    channel: Uuid,
    target: &str,
) -> Result<(), String> {
    let event = EventBuilder::new(Kind::Custom(9005), "")
        .tags([
            Tag::parse(["h", &channel.to_string()]).unwrap(),
            Tag::parse(["e", target]).unwrap(),
        ])
        .sign_with_keys(actor)
        .unwrap();
    send(state, tenant, event).await
}

/// Register `reader` as a connected client subscribed to `channel`; its
/// connection ID.
fn subscribe(state: &Arc<AppState>, tenant: &TenantContext, reader: &Keys, channel: Uuid) -> Uuid {
    let (conn, _rx) = test_conn(tenant, reader);
    let conn_id = conn.conn_id;
    let (tx, _rx) = tokio::sync::mpsc::channel(32);
    state.conn_manager.register(
        conn_id,
        tx,
        tokio::sync::mpsc::channel(4).0,
        tokio::sync::mpsc::channel(1).0,
        None,
        tokio_util::sync::CancellationToken::new(),
        tenant.community(),
        Arc::new(std::sync::atomic::AtomicU8::new(0)),
        Arc::new(tokio::sync::Mutex::new(Default::default())),
        3,
        crate::state::CommunityConnectionControl::new(tokio_util::sync::CancellationToken::new()),
    );
    state
        .conn_manager
        .set_authenticated_pubkey(conn_id, reader.public_key().to_bytes().to_vec());
    let filter = serde_json::json!({"kinds": [9], "#h": [channel.to_string()]});
    state.sub_registry.register_scoped(
        tenant.community(),
        conn_id,
        "comments".into(),
        vec![serde_json::from_value(filter).expect("filter")],
        Some(channel),
    );
    conn_id
}

fn subscribed(state: &AppState, tenant: &TenantContext, channel: Uuid, conn_id: Uuid) -> bool {
    state
        .sub_registry
        .channel_subscriber_conns_scoped(tenant.community(), channel)
        .contains(&conn_id)
}

/// Anyone who may post in a task's home opens its conversation, which is a
/// private `artifact` channel filed under the home that nobody owns. The
/// home's readers read and comment through the home; an outsider does
/// nothing. It is hidden from reads that name no channel. Opening is
/// idempotent, and only the derived address works.
/// Mutation: drop the derivation check in `open_conversation`, or the home
/// write check → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_home_reader_opens_and_comments_in_a_tasks_conversation() {
    let t = Tree::new("conversation-open").await;
    let (state, tenant) = (&t.state, &t.tenant);
    let (id, _) = create_task(state, tenant, &t.owner, t.child).await;

    let wrong = Uuid::new_v4();
    assert!(
        send(state, tenant, open_event(&t.reader, &id.to_string(), wrong))
            .await
            .is_err(),
        "only the derived address opens it"
    );
    assert!(
        open(state, tenant, &t.outsider, id).await.is_err(),
        "an outsider cannot open it"
    );
    let conversation = open(state, tenant, &t.reader, id)
        .await
        .expect("a project reader opens it");
    open(state, tenant, &t.owner, id)
        .await
        .expect("opening again is accepted");

    let channel = state
        .db
        .get_channel_for_event_write(tenant.community(), conversation)
        .await
        .expect("conversation");
    assert_eq!(channel.channel_type, "artifact");
    assert_eq!(channel.visibility, "private");
    assert_eq!(channel.parent_id, Some(t.child));
    assert_eq!(role_in(state, tenant, conversation, &t.reader).await, None);
    assert_eq!(role_in(state, tenant, conversation, &t.owner).await, None);

    let comment = post(state, tenant, &t.reader, conversation)
        .await
        .expect("a project reader comments");
    let reply = post(state, tenant, &t.owner, conversation)
        .await
        .expect("the home's owner comments");
    let mut both = vec![comment.clone(), reply.clone()];
    both.sort();
    for reader in [&t.reader, &t.owner] {
        let mut read = read_named(state, tenant, reader, conversation).await;
        read.sort();
        assert_eq!(read, both);
        assert_eq!(
            count_named(state, tenant, reader, conversation).await,
            Some(2)
        );
        let implicit = read_implicit(state, tenant, reader).await;
        assert!(
            !implicit.contains(&comment) && !implicit.contains(&reply),
            "comments are left out of reads that name no channel"
        );
    }
    assert!(read_named(state, tenant, &t.outsider, conversation)
        .await
        .is_empty());
    assert!(post(state, tenant, &t.outsider, conversation)
        .await
        .is_err());
}

/// A view-only home lets its project read a task's comments but not add any
/// or open a conversation.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_view_only_home_reads_comments_but_cannot_add_them() {
    let t = Tree::new("conversation-guest").await;
    let (state, tenant) = (&t.state, &t.tenant);
    let (id, _) = create_task(state, tenant, &t.owner, t.child).await;
    let (other, _) = create_task(state, tenant, &t.owner, t.child).await;
    let conversation = open(state, tenant, &t.owner, id).await.unwrap();
    let comment = post(state, tenant, &t.owner, conversation).await.unwrap();

    set_parent_access(state, tenant, &t.owner, t.child, "guest")
        .await
        .expect("view only");
    assert_eq!(
        read_named(state, tenant, &t.reader, conversation).await,
        vec![comment]
    );
    assert!(post(state, tenant, &t.reader, conversation).await.is_err());
    assert!(open(state, tenant, &t.reader, other).await.is_err());
}

/// Deleting a task closes its conversation to everyone and refuses opening
/// one; restoring it reopens the conversation.
/// Mutation: skip the `parent_access` change in `follow_artifact` → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn deleting_a_task_closes_its_comments_and_restoring_reopens_them() {
    let t = Tree::new("conversation-delete").await;
    let (state, tenant) = (&t.state, &t.tenant);
    let (id, created) = create_task(state, tenant, &t.owner, t.child).await;
    let conversation = open(state, tenant, &t.reader, id).await.unwrap();
    let comment = post(state, tenant, &t.reader, conversation).await.unwrap();
    let subscription = subscribe(state, tenant, &t.reader, conversation);

    let deleted = task(&t.owner, t.child, id, "delete", Some(&created));
    send(state, tenant, deleted.clone()).await.expect("delete");
    for person in [&t.reader, &t.owner] {
        assert!(read_named(state, tenant, person, conversation)
            .await
            .is_empty());
        assert!(post(state, tenant, person, conversation).await.is_err());
    }
    assert!(!subscribed(state, tenant, conversation, subscription));
    assert!(open(state, tenant, &t.owner, id).await.is_err());
    let (gone, gone_create) = create_task(state, tenant, &t.owner, t.child).await;
    send(
        state,
        tenant,
        task(&t.owner, t.child, gone, "delete", Some(&gone_create)),
    )
    .await
    .unwrap();
    assert!(
        open(state, tenant, &t.owner, gone).await.is_err(),
        "a deleted task's conversation cannot be opened"
    );

    send(
        state,
        tenant,
        task(&t.owner, t.child, id, "restore", Some(&deleted)),
    )
    .await
    .expect("restore");
    assert_eq!(
        read_named(state, tenant, &t.reader, conversation).await,
        vec![comment]
    );
    post(state, tenant, &t.reader, conversation)
        .await
        .expect("comments reopen");
}

/// Moving a task moves its conversation: the old home's readers lose it,
/// at once and live, and the new home's readers gain it. A move that would
/// put the conversation at a fourth level is refused, as is opening a
/// conversation for a task that lives there.
/// Mutation: skip the re-parent in `follow_artifact`, or the eviction after
/// it, or `refuse_conversation_home` → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn moving_a_task_moves_its_comments() {
    let t = Tree::new("conversation-move").await;
    let (state, tenant) = (&t.state, &t.tenant);
    let (id, created) = create_task(state, tenant, &t.owner, t.child).await;
    let conversation = open(state, tenant, &t.reader, id).await.unwrap();
    let comment = post(state, tenant, &t.reader, conversation).await.unwrap();
    let subscription = subscribe(state, tenant, &t.reader, conversation);

    let grandchild = create_channel(state, tenant, &t.owner, &[&["visibility", "private"]]).await;
    set_parent(state, tenant, &t.owner, grandchild, Some(t.child))
        .await
        .expect("file a grandchild");
    let too_deep = task(&t.owner, grandchild, id, "move", Some(&created));
    let refused = send(state, tenant, too_deep).await.unwrap_err();
    assert!(refused.contains("can't move there"), "{refused}");
    let (deep, _) = create_task(state, tenant, &t.owner, grandchild).await;
    let refused = open(state, tenant, &t.owner, deep).await.unwrap_err();
    assert!(
        refused.contains("comments aren't available here"),
        "{refused}"
    );

    let elsewhere = create_channel(state, tenant, &t.owner, &[&["visibility", "private"]]).await;
    let newcomer = Keys::generate();
    add(state, tenant, &t.owner, elsewhere, &newcomer, "member").await;
    assert!(read_named(state, tenant, &newcomer, conversation)
        .await
        .is_empty());
    send(
        state,
        tenant,
        task(&t.owner, elsewhere, id, "move", Some(&created)),
    )
    .await
    .expect("move");

    let channel = state
        .db
        .get_channel_for_event_write(tenant.community(), conversation)
        .await
        .unwrap();
    assert_eq!(channel.parent_id, Some(elsewhere));
    assert!(read_named(state, tenant, &t.reader, conversation)
        .await
        .is_empty());
    assert!(post(state, tenant, &t.reader, conversation).await.is_err());
    assert!(!subscribed(state, tenant, conversation, subscription));
    assert_eq!(
        read_named(state, tenant, &newcomer, conversation).await,
        vec![comment]
    );
    post(state, tenant, &newcomer, conversation)
        .await
        .expect("the new home's reader comments");
}

/// A conversation has no people or settings of its own; no other channel
/// may take its address or type, and it holds no artifacts.
/// Mutation: drop `conversation_refuses_kind`, or the generic 9007 guard
/// → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_conversation_has_no_people_or_settings() {
    let t = Tree::new("conversation-closed").await;
    let (state, tenant) = (&t.state, &t.tenant);
    let (id, _) = create_task(state, tenant, &t.owner, t.child).await;
    let conversation = open(state, tenant, &t.owner, id).await.unwrap();
    let h = conversation.to_string();
    let person = Keys::generate().public_key().to_hex();
    let attempts: [(u16, Vec<Vec<&str>>); 5] = [
        (9000, vec![vec!["p", &person]]),
        (9001, vec![vec!["p", &person]]),
        (9002, vec![vec!["name", "renamed"]]),
        (9008, vec![]),
        (9021, vec![]),
    ];
    for (kind, extra) in attempts {
        let mut tags = vec![Tag::parse(["h", h.as_str()]).unwrap()];
        tags.extend(extra.into_iter().map(|t| Tag::parse(t).unwrap()));
        let event = EventBuilder::new(Kind::Custom(kind), "")
            .tags(tags)
            .sign_with_keys(&t.owner)
            .unwrap();
        let refused = send(state, tenant, event).await.unwrap_err();
        assert!(
            refused.contains("no members or settings"),
            "{kind}: {refused}"
        );
    }

    let squat = |h: Uuid, channel_type: &str| {
        EventBuilder::new(Kind::Custom(9007), "")
            .tags([
                Tag::parse(["h", &h.to_string()]).unwrap(),
                Tag::parse(["name", "squat"]).unwrap(),
                Tag::parse(["channel_type", channel_type]).unwrap(),
            ])
            .sign_with_keys(&t.owner)
            .unwrap()
    };
    let (later, _) = create_task(state, tenant, &t.owner, t.child).await;
    let derived = buzz_core::artifact::conversation_id(&later.to_string());
    assert!(send(state, tenant, squat(derived, "stream")).await.is_err());
    assert!(send(state, tenant, squat(Uuid::new_v4(), "artifact"))
        .await
        .is_err());
    open(state, tenant, &t.owner, later)
        .await
        .expect("its address is still free");

    let refused = send(
        state,
        tenant,
        task(&t.owner, conversation, Uuid::new_v4(), "create", None),
    )
    .await
    .unwrap_err();
    assert!(
        refused.contains("cannot live in a conversation"),
        "{refused}"
    );
}

/// The home's own owners and admins delete comments; a member of the home
/// cannot delete someone else's, but may delete their own.
/// Mutation: moderate the conversation by its own rows only → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn the_homes_admins_moderate_comments() {
    let t = Tree::new("conversation-moderation").await;
    let (state, tenant) = (&t.state, &t.tenant);
    let member = Keys::generate();
    add(state, tenant, &t.owner, t.child, &member, "member").await;
    let (id, _) = create_task(state, tenant, &t.owner, t.child).await;
    let conversation = open(state, tenant, &t.owner, id).await.unwrap();
    let comment = post(state, tenant, &t.reader, conversation).await.unwrap();
    let own = post(state, tenant, &member, conversation).await.unwrap();

    assert!(moderate(state, tenant, &member, conversation, &comment)
        .await
        .is_err());
    moderate(state, tenant, &member, conversation, &own)
        .await
        .expect("an author deletes their own comment");
    moderate(state, tenant, &t.owner, conversation, &comment)
        .await
        .expect("the home's owner deletes a comment");
    assert!(read_named(state, tenant, &t.reader, conversation)
        .await
        .is_empty());
}

/// An auto-archiving chat outside any project holds task conversations
/// without becoming a project, and its comments close when it archives.
/// Mutation: count conversations in `has_live_children`, or drop the
/// home-archived check in ingest → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_auto_archiving_chat_holds_task_comments() {
    let t = Tree::new("conversation-ttl").await;
    let (state, tenant) = (&t.state, &t.tenant);
    let chat = create_channel(state, tenant, &t.owner, &[]).await;
    edit_channel(state, tenant, &t.owner, chat, &[&["ttl", "3600"]])
        .await
        .expect("an auto-archiving chat");
    let (id, _) = create_task(state, tenant, &t.owner, chat).await;
    let conversation = open(state, tenant, &t.owner, id)
        .await
        .expect("its task's comments open");
    post(state, tenant, &t.owner, conversation).await.unwrap();
    edit_channel(state, tenant, &t.owner, chat, &[&["ttl", "7200"]])
        .await
        .expect("a conversation does not make the chat a project");

    auto_archive(state, tenant, chat).await;
    let refused = post(state, tenant, &t.owner, conversation)
        .await
        .unwrap_err();
    assert!(refused.contains("archived"), "{refused}");
    assert_eq!(
        read_named(state, tenant, &t.owner, conversation)
            .await
            .len(),
        1,
        "comments stay readable"
    );
}
