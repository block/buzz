//! Postgres-backed tests for who a task names (NOSTR.md, NIP-AR Assignees
//! and Creator) and for finding tasks by them. Saves go through real
//! ingest, reads through the real `POST /query` route.

use std::sync::Arc;

use buzz_core::TenantContext;
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::handlers::artifact_conversation_postgres_tests::send;
use crate::handlers::channel_access_postgres_tests::{live_state, query};
use crate::handlers::group_state_postgres_tests::{community, create_channel};
use crate::handlers::parent_access_postgres_tests::{add, set_parent};
use crate::state::AppState;

/// A `buzz.task` revision of `id` in `home`, signed by `key`, created at
/// `at`, with `extra` tags.
fn revision(
    key: &Keys,
    home: Uuid,
    id: Uuid,
    op: &str,
    prev: Option<&Event>,
    extra: &[[&str; 2]],
    at: u64,
) -> Event {
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
    tags.extend(
        extra
            .iter()
            .map(|[n, v]| vec![n.to_string(), v.to_string()]),
    );
    EventBuilder::new(Kind::Custom(45010), if op == "delete" { "" } else { "x" })
        .tags(tags.into_iter().map(|t| Tag::parse(t).unwrap()))
        .custom_created_at(nostr::Timestamp::from(at))
        .sign_with_keys(key)
        .unwrap()
}

fn now() -> u64 {
    nostr::Timestamp::now().as_secs()
}

/// The project, a child and a grandchild filed under it, all private and
/// owned by `owner`; `member` is a member of the project only; `elsewhere`
/// is a private channel only `owner` reads.
struct Fixture {
    state: Arc<AppState>,
    tenant: TenantContext,
    owner: Keys,
    member: Keys,
    project: Uuid,
    child: Uuid,
    grandchild: Uuid,
    elsewhere: Uuid,
}

impl Fixture {
    async fn new(label: &str) -> Self {
        let state = live_state().await;
        let tenant = community(&state, label).await;
        let owner = Keys::generate();
        let member = Keys::generate();
        let private: &[&[&str]] = &[&["visibility", "private"]];
        let project = create_channel(&state, &tenant, &owner, private).await;
        let child = create_channel(&state, &tenant, &owner, private).await;
        let grandchild = create_channel(&state, &tenant, &owner, private).await;
        let elsewhere = create_channel(&state, &tenant, &owner, private).await;
        set_parent(&state, &tenant, &owner, child, Some(project))
            .await
            .expect("file the child");
        set_parent(&state, &tenant, &owner, grandchild, Some(child))
            .await
            .expect("file the grandchild");
        add(&state, &tenant, &owner, project, &member, "member").await;
        Self {
            state,
            tenant,
            owner,
            member,
            project,
            child,
            grandchild,
            elsewhere,
        }
    }

    async fn save(&self, event: &Event) -> Result<(), String> {
        send(&self.state, &self.tenant, event.clone()).await
    }

    /// The `d` of each artifact `viewer` finds with `filter`, in order.
    async fn find(&self, viewer: &Keys, filter: Value) -> Vec<String> {
        let (status, body) = query(&self.state, &self.tenant, viewer, &json!([filter]), true).await;
        assert!(status.is_success(), "{filter}: {status} {body}");
        body.as_array()
            .unwrap()
            .iter()
            .map(|e| {
                e["tags"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|t| t[0] == "d")
                    .unwrap()[1]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect()
    }
}

/// `r` is refused under reason `code`, naming exactly `keys`.
fn refused(r: Result<(), String>, code: &str, keys: &[&str]) {
    let e = r.expect_err("refused");
    assert!(
        e.contains(&format!("invalid: {code}:{} — ", keys.join(","))),
        "{code}: {e}"
    );
}

impl Fixture {
    /// A fresh key recorded as an agent of `owner`, made a member of the
    /// project so only its being an agent can refuse it.
    async fn agent(&self) -> Keys {
        let agent = Keys::generate();
        let community = self.tenant.community();
        for key in [&agent, &self.owner] {
            self.state
                .db
                .ensure_user(community, key.public_key().as_bytes())
                .await
                .expect("ensure user");
        }
        self.state
            .db
            .set_agent_owner(
                community,
                agent.public_key().as_bytes(),
                self.owner.public_key().as_bytes(),
            )
            .await
            .expect("record the owner");
        add(
            &self.state,
            &self.tenant,
            &self.owner,
            self.project,
            &agent,
            "member",
        )
        .await;
        agent
    }

    /// Forget the head's creator and assignees, as a head saved before the
    /// relay kept them.
    async fn as_before_0096(&self, id: Uuid) {
        sqlx::query("UPDATE artifact_heads SET creator=NULL, assignees=NULL WHERE community_id=$1 AND artifact_id=$2")
            .bind(self.tenant.community().as_uuid())
            .bind(id)
            .execute(self.state.db.pool())
            .await
            .expect("forget the head's creator and assignees");
    }

    /// Expire a stored payload, as retention may.
    async fn expire(&self, event: &Event) {
        sqlx::query("DELETE FROM events WHERE community_id=$1 AND id=$2")
            .bind(self.tenant.community().as_uuid())
            .bind(event.id.as_bytes().as_slice())
            .execute(self.state.db.pool())
            .await
            .expect("expire a payload");
    }

    async fn head(&self, id: Uuid) -> (Option<String>, Option<Vec<String>>) {
        sqlx::query_as("SELECT creator, assignees FROM artifact_heads WHERE community_id=$1 AND artifact_id=$2")
            .bind(self.tenant.community().as_uuid())
            .bind(id)
            .fetch_one(self.state.db.pool())
            .await
            .expect("head")
    }

    async fn remove_from_project(&self, person: &Keys) {
        let removal = EventBuilder::new(Kind::Custom(9001), "")
            .tags([
                Tag::parse(["h", &self.project.to_string()]).unwrap(),
                Tag::parse(["p", &person.public_key().to_hex()]).unwrap(),
            ])
            .sign_with_keys(&self.owner)
            .unwrap();
        self.save(&removal).await.expect("remove from the project");
    }
}

/// Every assignee must be able to read the home a save puts the task in.
/// A create, move or restore checks everyone; an update checks only the
/// people it adds, so someone who lost access since doesn't block an
/// unrelated edit. The refusal names who failed. At most four, people only,
/// and an agent can't sign the save.
/// Mutation: skip the assignee check, check everyone on an update, check
/// nobody on a move or restore, raise the cap, or skip either agent check
/// → RED.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn every_assignee_must_see_the_task() {
    let f = Fixture::new("assignees").await;
    let member = f.member.public_key().to_hex();
    let outsider = Keys::generate().public_key().to_hex();
    let id = Uuid::new_v4();

    refused(
        f.save(&revision(
            &f.owner,
            f.child,
            id,
            "create",
            None,
            &[["assignee", &outsider]],
            now(),
        ))
        .await,
        "assignee-unreadable",
        &[&outsider],
    );
    let create = revision(
        &f.owner,
        f.child,
        id,
        "create",
        None,
        &[["assignee", &member]],
        now(),
    );
    f.save(&create)
        .await
        .expect("the member reads the child through the project");
    refused(
        f.save(&revision(
            &f.owner,
            f.child,
            id,
            "update",
            Some(&create),
            &[["assignee", &member], ["assignee", &outsider]],
            now(),
        ))
        .await,
        "assignee-unreadable",
        &[&outsider],
    );
    refused(
        f.save(&revision(
            &f.owner,
            f.elsewhere,
            id,
            "move",
            Some(&create),
            &[["assignee", &member]],
            now(),
        ))
        .await,
        "assignee-unreadable",
        &[&member],
    );
    let moved = revision(&f.owner, f.elsewhere, id, "move", Some(&create), &[], now());
    f.save(&moved).await.expect("a move without the member");
    refused(
        f.save(&revision(
            &f.owner,
            f.elsewhere,
            id,
            "update",
            Some(&moved),
            &[["assignee", &member]],
            now(),
        ))
        .await,
        "assignee-unreadable",
        &[&member],
    );

    // Access lost after assignment: an edit that keeps them is accepted, a
    // move or restore that keeps them is refused, a delete names nobody.
    let other = Uuid::new_v4();
    let created = revision(
        &f.owner,
        f.grandchild,
        other,
        "create",
        None,
        &[["assignee", &member]],
        now(),
    );
    f.save(&created).await.expect("assigned in the grandchild");
    f.remove_from_project(&f.member).await;
    // Measured against the stored current state, even once retention has
    // expired the revision that assigned them.
    f.expire(&created).await;
    let edit = revision(
        &f.owner,
        f.grandchild,
        other,
        "update",
        Some(&created),
        &[["assignee", &member]],
        now(),
    );
    f.save(&edit)
        .await
        .expect("an edit that adds nobody isn't held to a lost access");
    refused(
        f.save(&revision(
            &f.owner,
            f.child,
            other,
            "move",
            Some(&edit),
            &[["assignee", &member]],
            now(),
        ))
        .await,
        "assignee-unreadable",
        &[&member],
    );
    let deleted = revision(
        &f.owner,
        f.grandchild,
        other,
        "delete",
        Some(&edit),
        &[],
        now(),
    );
    f.save(&deleted).await.expect("a delete names nobody");
    refused(
        f.save(&revision(
            &f.owner,
            f.grandchild,
            other,
            "restore",
            Some(&deleted),
            &[["assignee", &member]],
            now(),
        ))
        .await,
        "assignee-unreadable",
        &[&member],
    );

    // At most four, people only, signed by a person.
    let five: Vec<String> = (0..5)
        .map(|_| Keys::generate().public_key().to_hex())
        .collect();
    let tags: Vec<[&str; 2]> = five.iter().map(|k| ["assignee", k.as_str()]).collect();
    refused(
        f.save(&revision(
            &f.owner,
            f.child,
            Uuid::new_v4(),
            "create",
            None,
            &tags,
            now(),
        ))
        .await,
        "too-many-assignees",
        &[],
    );
    let agent = f.agent().await;
    let agent_hex = agent.public_key().to_hex();
    refused(
        f.save(&revision(
            &f.owner,
            f.child,
            Uuid::new_v4(),
            "create",
            None,
            &[["assignee", &agent_hex]],
            now(),
        ))
        .await,
        "assignee-agent",
        &[&agent_hex],
    );
    refused(
        f.save(&revision(
            &agent,
            f.child,
            Uuid::new_v4(),
            "create",
            None,
            &[],
            now(),
        ))
        .await,
        "signer-agent",
        &[&agent_hex],
    );
}

/// The creator is the creating signer and never changes, through moves and
/// a delete and restore, even after retention expires the earlier payloads.
/// A refusal names the creator to resend with.
/// Mutation: skip the create check, skip the keep check, or let a later
/// revision rewrite the head's creator → RED.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn the_creator_is_the_signer_and_never_changes() {
    let f = Fixture::new("creator").await;
    let owner = f.owner.public_key().to_hex();
    let member = f.member.public_key().to_hex();
    let id = Uuid::new_v4();

    refused(
        f.save(&revision(
            &f.owner,
            f.child,
            id,
            "create",
            None,
            &[["creator", &member]],
            now(),
        ))
        .await,
        "creator-not-signer",
        &[&member],
    );
    let create = revision(
        &f.owner,
        f.child,
        id,
        "create",
        None,
        &[["creator", &owner]],
        now(),
    );
    f.save(&create).await.expect("the signer creates");
    for creator in [None, Some(member.as_str())] {
        let tags: Vec<[&str; 2]> = creator.map(|c| ["creator", c]).into_iter().collect();
        refused(
            f.save(&revision(
                &f.member,
                f.child,
                id,
                "update",
                Some(&create),
                &tags,
                now(),
            ))
            .await,
            "creator-changed",
            &[&owner],
        );
    }
    let edit = revision(
        &f.member,
        f.child,
        id,
        "update",
        Some(&create),
        &[["creator", &owner]],
        now(),
    );
    f.save(&edit)
        .await
        .expect("another editor keeps the creator");
    let moved = revision(
        &f.owner,
        f.project,
        id,
        "move",
        Some(&edit),
        &[["creator", &owner]],
        now(),
    );
    f.save(&moved).await.expect("a move keeps the creator");
    let deleted = revision(&f.owner, f.project, id, "delete", Some(&moved), &[], now());
    f.save(&deleted).await.expect("a delete carries no creator");
    for expired in [&create, &edit, &moved] {
        f.expire(expired).await;
    }
    refused(
        f.save(&revision(
            &f.owner,
            f.project,
            id,
            "restore",
            Some(&deleted),
            &[],
            now(),
        ))
        .await,
        "creator-changed",
        &[&owner],
    );
    f.save(&revision(
        &f.owner,
        f.project,
        id,
        "restore",
        Some(&deleted),
        &[["creator", &owner]],
        now(),
    ))
    .await
    .expect("a restore keeps the creator from before the delete");

    // Without a creator at create, none can be added later.
    let plain = Uuid::new_v4();
    let create = revision(&f.owner, f.child, plain, "create", None, &[], now());
    f.save(&create).await.expect("no creator");
    refused(
        f.save(&revision(
            &f.owner,
            f.child,
            plain,
            "update",
            Some(&create),
            &[["creator", &owner]],
            now(),
        ))
        .await,
        "creator-changed",
        &["none"],
    );
}

/// A task saved before the relay kept its creator and assignees gets them
/// on its next save: the creator from its first save's signer, else the
/// current revision's tag, else none, which nobody can claim later; the
/// assignees from its current revision, so an edit isn't held to someone
/// who lost access since. It never becomes uneditable.
/// Mutation: treat unknown assignees as nobody assigned, take the current
/// tag over the first signer, or let an edit set an unknown creator → RED.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn a_task_saved_before_0096_stays_editable() {
    let f = Fixture::new("before-0096").await;
    let owner = f.owner.public_key().to_hex();
    let member = f.member.public_key().to_hex();

    // Its assignee lost access; a title edit is still accepted.
    let id = Uuid::new_v4();
    let create = revision(
        &f.owner,
        f.grandchild,
        id,
        "create",
        None,
        &[["creator", &owner], ["assignee", &member]],
        now(),
    );
    f.save(&create).await.expect("create");
    f.as_before_0096(id).await;
    f.remove_from_project(&f.member).await;
    f.save(&revision(
        &f.owner,
        f.grandchild,
        id,
        "update",
        Some(&create),
        &[["creator", &owner], ["assignee", &member]],
        now(),
    ))
    .await
    .expect("a title edit keeps the assignee who lost access");
    assert_eq!(
        f.head(id).await,
        (Some(owner.clone()), Some(vec![member.clone()]))
    );

    // The first save's signer outranks a current tag; the refusal names it.
    let id = Uuid::new_v4();
    let editor = Keys::generate();
    add(&f.state, &f.tenant, &f.owner, f.child, &editor, "member").await;
    let create = revision(&f.owner, f.child, id, "create", None, &[], now());
    f.save(&create).await.expect("create without a tag");
    let edit = revision(&editor, f.child, id, "update", Some(&create), &[], now());
    f.save(&edit).await.expect("an edit by someone else");
    f.as_before_0096(id).await;
    // Before the relay checked it, a stored tag could name anyone.
    sqlx::query("UPDATE events SET tags = tags || jsonb_build_array(jsonb_build_array('creator', $3::text)) WHERE community_id=$1 AND id=$2")
        .bind(f.tenant.community().as_uuid())
        .bind(edit.id.as_bytes().as_slice())
        .bind(&member)
        .execute(f.state.db.pool())
        .await
        .expect("an unverified tag");
    for tags in [vec![], vec![["creator", member.as_str()]]] {
        refused(
            f.save(&revision(
                &f.owner,
                f.child,
                id,
                "update",
                Some(&edit),
                &tags,
                now(),
            ))
            .await,
            "creator-changed",
            &[&owner],
        );
    }
    f.save(&revision(
        &f.owner,
        f.child,
        id,
        "update",
        Some(&edit),
        &[["creator", &owner]],
        now(),
    ))
    .await
    .expect("resent with the named creator");

    // First save gone: the current revision's tag.
    let id = Uuid::new_v4();
    let create = revision(
        &f.owner,
        f.project,
        id,
        "create",
        None,
        &[["creator", &owner]],
        now(),
    );
    f.save(&create).await.expect("create");
    let edit = revision(
        &f.owner,
        f.project,
        id,
        "update",
        Some(&create),
        &[["creator", &owner]],
        now(),
    );
    f.save(&edit).await.expect("edit");
    f.as_before_0096(id).await;
    f.expire(&create).await;
    refused(
        f.save(&revision(
            &f.owner,
            f.project,
            id,
            "update",
            Some(&edit),
            &[["creator", &member]],
            now(),
        ))
        .await,
        "creator-changed",
        &[&owner],
    );
    f.save(&revision(
        &f.owner,
        f.project,
        id,
        "update",
        Some(&edit),
        &[["creator", &owner]],
        now(),
    ))
    .await
    .expect("the current revision's creator");

    // Neither on hand: no creator, and nobody can claim one.
    let id = Uuid::new_v4();
    let create = revision(&f.owner, f.child, id, "create", None, &[], now());
    f.save(&create).await.expect("create");
    f.as_before_0096(id).await;
    f.expire(&create).await;
    refused(
        f.save(&revision(
            &f.owner,
            f.child,
            id,
            "update",
            Some(&create),
            &[["creator", &owner]],
            now(),
        ))
        .await,
        "creator-changed",
        &["none"],
    );
    f.save(&revision(
        &f.owner,
        f.child,
        id,
        "update",
        Some(&create),
        &[],
        now(),
    ))
    .await
    .expect("an edit without a creator");
    assert_eq!(f.head(id).await, (None, Some(vec![])));
}

/// Find reads current tasks by assignee and status, finds the ones nobody
/// has, covers a project's whole tree with `within`, and pages newest
/// first with `until`, all inside what the reader may read.
/// Mutation: drop the `unassigned` clause, drop the grandchild level from
/// `within`, keep the received order under `until`, or let an incremental
/// read take them → RED.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn find_reads_unassigned_tasks_within_a_project_page_by_page() {
    let f = Fixture::new("find").await;
    let member = f.member.public_key().to_hex();
    let t = now();
    let mut ids = Vec::new();
    // Saved newest first, so the received order is the reverse of
    // `created_at` order.
    for (home, at, extra) in [
        (f.elsewhere, t - 10, vec![["status", "todo"]]),
        (f.project, t - 20, vec![["status", "done"]]),
        (f.grandchild, t - 30, vec![["status", "in_progress"]]),
        (
            f.child,
            t - 40,
            vec![["status", "todo"], ["assignee", member.as_str()]],
        ),
    ] {
        let id = Uuid::new_v4();
        f.save(&revision(&f.owner, home, id, "create", None, &extra, at))
            .await
            .expect("create");
        ids.push(id.to_string());
    }
    let [elsewhere, done, open, assigned] = <[String; 4]>::try_from(ids).unwrap();
    let open_statuses = json!(["todo", "in_progress"]);

    assert_eq!(
        f.find(
            &f.member,
            json!({"artifact":"current","#type":["buzz.task"],"#assignee":[member]})
        )
        .await,
        vec![assigned.clone()],
    );
    assert_eq!(
        f.find(
            &f.member,
            json!({"artifact":"current","#type":["buzz.task"],"#status":open_statuses,"unassigned":true}),
        )
        .await,
        vec![open.clone()],
        "open and nobody has it, among what the member reads"
    );
    assert_eq!(
        f.find(
            &f.owner,
            json!({"artifact":"current","#status":open_statuses,"unassigned":true,"until":t}),
        )
        .await,
        vec![elsewhere.clone(), open.clone()],
    );
    assert_eq!(
        f.find(
            &f.owner,
            json!({"artifact":"current","within":f.project,"until":t})
        )
        .await,
        vec![done.clone(), open.clone(), assigned.clone()],
        "the project, its child and its grandchild, newest first"
    );
    assert_eq!(
        f.find(
            &f.owner,
            json!({"artifact":"current","within":f.child,"until":t})
        )
        .await,
        vec![open.clone(), assigned.clone()],
    );
    assert!(f
        .find(
            &Keys::generate(),
            json!({"artifact":"current","within":f.project})
        )
        .await
        .is_empty());

    let page =
        |until: u64| json!({"artifact":"current","within":f.project,"until":until,"limit":2});
    assert_eq!(
        f.find(&f.owner, page(t)).await,
        vec![done.clone(), open.clone()]
    );
    // `until` is inclusive: the next page starts at the oldest one got.
    assert_eq!(
        f.find(&f.owner, page(t - 30)).await,
        vec![open.clone(), assigned.clone()]
    );
    assert_eq!(f.find(&f.owner, page(t - 31)).await, vec![assigned.clone()]);
    assert!(f.find(&f.owner, page(t - 41)).await.is_empty());

    // Incremental reads have no form of these filters, so they are refused
    // there rather than ignored.
    let plain = json!([{"artifact": "incremental", "query": {"artifact": "current", "#type": ["buzz.task"]}}]);
    let (status, response) = query(&f.state, &f.tenant, &f.owner, &plain, true).await;
    assert!(status.is_success(), "{status}: {response}");
    for extra in [
        json!({"unassigned": true}),
        json!({"within": f.project}),
        json!({"until": t}),
    ] {
        let mut inner = json!({"artifact": "current", "#type": ["buzz.task"]});
        inner
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let body = json!([{"artifact": "incremental", "query": inner}]);
        let (status, response) = query(&f.state, &f.tenant, &f.owner, &body, true).await;
        assert_eq!(
            status,
            axum::http::StatusCode::BAD_REQUEST,
            "{extra}: {response}"
        );
    }
}
