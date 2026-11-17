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

/// Every assignee must be able to read the home a save puts the task in:
/// on create, on edit and on move, judged now, not when they were added.
/// A delete names nobody.
/// Mutation: skip the assignee check, or check the move's source instead of
/// its destination → RED.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn every_assignee_must_see_the_task() {
    let f = Fixture::new("assignees").await;
    let member = f.member.public_key().to_hex();
    let outsider = Keys::generate().public_key().to_hex();
    let refused = |r: Result<(), String>| {
        let e = r.expect_err("refused");
        assert!(e.contains("an assignee can't see this artifact"), "{e}");
    };
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
    );

    // Judged now: once the member leaves the project, a save naming them
    // is refused.
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
    let removal = EventBuilder::new(Kind::Custom(9001), "")
        .tags([
            Tag::parse(["h", &f.project.to_string()]).unwrap(),
            Tag::parse(["p", &member]).unwrap(),
        ])
        .sign_with_keys(&f.owner)
        .unwrap();
    f.save(&removal).await.expect("remove the member");
    refused(
        f.save(&revision(
            &f.owner,
            f.grandchild,
            other,
            "update",
            Some(&created),
            &[["assignee", &member]],
            now(),
        ))
        .await,
    );
    f.save(&revision(
        &f.owner,
        f.grandchild,
        other,
        "delete",
        Some(&created),
        &[],
        now(),
    ))
    .await
    .expect("a delete names nobody");
}

/// A creator tag names the creating signer, and every later revision keeps
/// it, through moves and a delete and restore, even after retention expires
/// the earlier payloads, so the current revision's creator can be trusted
/// without its history.
/// Mutation: skip the create check, skip the keep check, or let a later
/// revision rewrite the head's creator → RED.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn the_creator_is_the_signer_and_never_changes() {
    let f = Fixture::new("creator").await;
    let owner = f.owner.public_key().to_hex();
    let member = f.member.public_key().to_hex();
    let id = Uuid::new_v4();
    let refused = |r: Result<(), String>, reason: &str| {
        let e = r.expect_err("refused");
        assert!(e.contains(reason), "{e}");
    };

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
        "creator must be the signer on create",
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
            "creator cannot change",
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
    // Retention may expire every earlier payload; the rule still holds.
    for expired in [&create, &edit, &moved] {
        sqlx::query("DELETE FROM events WHERE community_id=$1 AND id=$2")
            .bind(f.tenant.community().as_uuid())
            .bind(expired.id.as_bytes().as_slice())
            .execute(f.state.db.pool())
            .await
            .expect("expire a payload");
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
        "creator cannot change",
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
        "creator cannot change",
    );
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
