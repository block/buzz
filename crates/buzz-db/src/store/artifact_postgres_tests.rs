//! PostgreSQL regression tests for the production artifact transaction/query seams.
use super::artifact::{ArtifactOutcome, FeedbackWakeOutcome};
use crate::Db;
use buzz_core::{artifact, CommunityId};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use sqlx::PgPool;
use uuid::Uuid;

struct Fixture {
    db: Db,
    community: CommunityId,
    a: Uuid,
    b: Uuid,
    owner: Keys,
    peer: Keys,
    relay: Keys,
}
impl Fixture {
    async fn new() -> Self {
        let pool = PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() != Ok("desired") {
            crate::migration::run_migrations(&pool).await.unwrap();
        }
        let db = Db::from_pool(pool);
        db.ensure_future_partitions(1, true).await.unwrap();
        let community = CommunityId::from_uuid(Uuid::new_v4());
        sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
            .bind(community.as_uuid())
            .bind(format!("artifact-{}.test", community.as_uuid()))
            .execute(&db.pool)
            .await
            .unwrap();
        let owner = Keys::generate();
        let peer = Keys::generate();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        for (id, visibility) in [(a, "open"), (b, "private")] {
            sqlx::query("INSERT INTO channels(community_id,id,name,visibility,channel_type,created_by) VALUES($1,$2,$3,$4::channel_visibility,'stream',$5)")
                .bind(community.as_uuid()).bind(id).bind(id.to_string()).bind(visibility).bind(owner.public_key().to_bytes().as_slice()).execute(&db.pool).await.unwrap();
            sqlx::query("INSERT INTO channel_members(community_id,channel_id,pubkey,role) VALUES($1,$2,$3,'owner')")
                .bind(community.as_uuid()).bind(id).bind(owner.public_key().to_bytes().as_slice()).execute(&db.pool).await.unwrap();
        }
        Self {
            db,
            community,
            a,
            b,
            owner,
            peer,
            relay: Keys::generate(),
        }
    }
    fn revision(
        &self,
        d: Uuid,
        op: &str,
        home: Uuid,
        prev: Option<&Event>,
        key: &Keys,
        extra: Vec<Vec<String>>,
    ) -> Event {
        let mut tags = vec![
            vec!["ar".into(), "1".into()],
            vec!["d".into(), d.to_string()],
            vec!["h".into(), home.to_string()],
            vec!["type".into(), "buzz.task".into()],
            vec!["op".into(), op.into()],
        ];
        if op != "delete" {
            tags.push(vec!["title".into(), "Test".into()]);
        }
        if let Some(e) = prev {
            tags.push(vec!["prev".into(), e.id.to_hex()]);
        }
        tags.extend(extra);
        EventBuilder::new(
            Kind::Custom(45010),
            if op == "delete" { "" } else { "secret" },
        )
        .tags(tags.into_iter().map(|t| Tag::parse(t).unwrap()))
        .sign_with_keys(key)
        .unwrap()
    }
    async fn accept(&self, e: &Event, source: Option<Uuid>) -> ArtifactOutcome {
        let env = artifact::validate(e).unwrap();
        self.db
            .accept_artifact(self.community, e, &env, source, &self.relay)
            .await
            .unwrap()
    }
    async fn count(&self, key: &Keys, value: serde_json::Value) -> i64 {
        self.query(key, value, true).await.1
    }
    async fn query(
        &self,
        key: &Keys,
        value: serde_json::Value,
        count: bool,
    ) -> (Vec<buzz_core::StoredEvent>, i64) {
        self.db
            .query_artifacts(
                self.community,
                key.public_key().as_bytes(),
                &artifact::parse_query(&value).unwrap(),
                count,
            )
            .await
            .unwrap()
    }
}
#[tokio::test]
#[ignore = "requires Postgres"]
async fn lifecycle_cas_queries_move_redaction_and_retention() {
    let f = Fixture::new().await;
    let d = Uuid::new_v4();
    let project = |v: &str| vec![vec!["project".to_string(), v.to_string()]];
    let create = f.revision(d, "create", f.a, None, &f.owner, project("A"));
    assert!(matches!(
        f.accept(&create, None).await,
        ArtifactOutcome::Accepted(_)
    ));
    let x = f.revision(d, "update", f.a, Some(&create), &f.owner, project("B"));
    let y = f.revision(d, "update", f.a, Some(&create), &f.peer, project("B"));
    let (rx, ry) = tokio::join!(f.accept(&x, None), f.accept(&y, None));
    let head = match (rx, ry) {
        (ArtifactOutcome::Accepted(_), ArtifactOutcome::Conflict(..)) => &x,
        (ArtifactOutcome::Conflict(..), ArtifactOutcome::Accepted(_)) => &y,
        other => panic!("exactly one CAS winner: {other:?}"),
    };
    assert!(matches!(
        f.accept(&create, None).await,
        ArtifactOutcome::Duplicate
    ));
    let current = |p: &str| serde_json::json!({"artifact":"current","#d":[d],"#project":[p]});
    assert_eq!(f.count(&f.owner, current("A")).await, 0);
    assert_eq!(f.count(&f.owner, current("B")).await, 1);
    // First value in the same tag, never an annotation or reverse-name match.
    let wrong = f.revision(
        d,
        "update",
        f.a,
        Some(head),
        &f.owner,
        vec![
            vec!["project".into(), "C".into(), "B".into()],
            vec!["B".into(), "project".into()],
        ],
    );
    assert!(matches!(
        f.accept(&wrong, None).await,
        ArtifactOutcome::Accepted(_)
    ));
    assert_eq!(f.count(&f.owner, current("B")).await, 0);

    // A move commits only against the source the relay authorized.
    let moved = f.revision(d, "move", f.b, Some(&wrong), &f.owner, vec![]);
    assert!(matches!(
        f.accept(&moved, Some(f.b)).await,
        ArtifactOutcome::Conflict(..)
    ));
    let ArtifactOutcome::Accepted(stored) = f.accept(&moved, Some(f.a)).await else {
        panic!("move accepted");
    };
    let removal = &stored[1];
    assert_eq!(removal.event.kind.as_u16(), 45011);
    assert_eq!(removal.channel_id, Some(f.a));
    assert!(!serde_json::to_string(&removal.event)
        .unwrap()
        .contains(&f.b.to_string()));
    let all = serde_json::json!({"artifact":"current","#d":[d]});
    let history = serde_json::json!({"artifact":"history","#d":[d]});
    assert_eq!(f.count(&f.peer, all.clone()).await, 0);
    assert_eq!(f.count(&f.peer, history.clone()).await, 3);
    assert_eq!(f.count(&f.owner, history.clone()).await, 4);

    // Expiring an earlier payload keeps its identity reserved.
    let retention = |id: nostr::EventId| {
        sqlx::query("DELETE FROM events WHERE community_id=$1 AND id=$2")
            .bind(f.community.as_uuid())
            .bind(id.as_bytes().to_vec())
    };
    assert_eq!(
        retention(create.id)
            .execute(&f.db.pool)
            .await
            .unwrap()
            .rows_affected(),
        1
    );
    assert!(matches!(
        f.accept(&create, None).await,
        ArtifactOutcome::Duplicate
    ));

    // Redaction is the generic soft delete: the head stays, its payload is hidden
    // everywhere and may then be purged.
    assert!(f
        .db
        .soft_delete_event_and_update_thread(f.community, moved.id.as_bytes(), None, None)
        .await
        .unwrap());
    assert_eq!(f.count(&f.owner, all.clone()).await, 0);
    assert_eq!(f.count(&f.owner, history.clone()).await, 2);
    assert_eq!(
        retention(moved.id)
            .execute(&f.db.pool)
            .await
            .unwrap()
            .rows_affected(),
        1
    );
    assert!(f
        .db
        .soft_delete_event_and_update_thread(f.community, removal.event.id.as_bytes(), None, None)
        .await
        .is_err());

    let deleted = f.revision(d, "delete", f.b, Some(&moved), &f.owner, vec![]);
    assert!(matches!(
        f.accept(&deleted, None).await,
        ArtifactOutcome::Accepted(_)
    ));
    assert_eq!(f.count(&f.owner, all.clone()).await, 0);
    let update = f.revision(d, "update", f.b, Some(&deleted), &f.owner, vec![]);
    assert!(matches!(
        f.accept(&update, None).await,
        ArtifactOutcome::Rejected(_)
    ));
    let restore = f.revision(d, "restore", f.b, Some(&deleted), &f.peer, vec![]);
    assert!(matches!(
        f.accept(&restore, None).await,
        ArtifactOutcome::Accepted(_)
    ));
    assert_eq!(f.count(&f.owner, all.clone()).await, 1);
    // Reads observe removed private membership.
    sqlx::query(
        "UPDATE channel_members SET removed_at=now() WHERE community_id=$1 AND channel_id=$2",
    )
    .bind(f.community.as_uuid())
    .bind(f.b)
    .execute(&f.db.pool)
    .await
    .unwrap();
    assert_eq!(f.count(&f.owner, all).await, 0);
    f.db.validate_deletion_serving_catalog().await.unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn repeated_moves_get_distinct_source_safe_markers() {
    let f = Fixture::new().await;
    let d = Uuid::new_v4();
    let create = f.revision(d, "create", f.a, None, &f.owner, vec![]);
    let there = f.revision(d, "move", f.b, Some(&create), &f.owner, vec![]);
    let back = f.revision(d, "move", f.a, Some(&there), &f.owner, vec![]);
    let again = f.revision(d, "move", f.b, Some(&back), &f.owner, vec![]);
    assert!(matches!(
        f.accept(&create, None).await,
        ArtifactOutcome::Accepted(_)
    ));
    let mut markers = Vec::new();
    for (event, source, replaced) in [
        (&there, f.a, &create),
        (&back, f.b, &there),
        (&again, f.a, &back),
    ] {
        let ArtifactOutcome::Accepted(stored) = f.accept(event, Some(source)).await else {
            panic!("move accepted");
        };
        let marker = stored[1].event.clone();
        // Only `prev` distinguishes same-second markers for one source.
        let tags: Vec<_> = marker.tags.iter().map(|t| t.as_slice().to_vec()).collect();
        assert_eq!(
            tags,
            [
                vec!["ar".to_string(), "1".into()],
                vec!["d".into(), d.to_string()],
                vec!["h".into(), source.to_string()],
                vec!["reason".into(), "moved".into()],
                vec!["prev".into(), replaced.id.to_hex()],
            ]
        );
        markers.push(marker.id);
    }
    assert_ne!(markers[0], markers[2]);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn root_anchor_and_tenant_boundaries() {
    let f = Fixture::new().await;
    let d = Uuid::new_v4();
    let invalid = f.revision(
        d,
        "create",
        f.a,
        None,
        &f.owner,
        vec![vec!["root".into(), "a".repeat(64)]],
    );
    assert!(matches!(
        f.accept(&invalid, None).await,
        ArtifactOutcome::Rejected(_)
    ));
    let anchor = EventBuilder::new(Kind::Custom(9), "anchor")
        .tags([Tag::parse(["h", &f.a.to_string()]).unwrap()])
        .sign_with_keys(&f.owner)
        .unwrap();
    f.db.insert_event(f.community, &anchor, Some(f.a))
        .await
        .unwrap();
    let root = || vec![vec!["root".to_string(), anchor.id.to_hex()]];
    let create = f.revision(d, "create", f.a, None, &f.owner, root());
    assert!(matches!(
        f.accept(&create, None).await,
        ArtifactOutcome::Accepted(_)
    ));
    // Later loss of the anchor does not block edits that keep it.
    sqlx::query("UPDATE events SET deleted_at=now() WHERE community_id=$1 AND id=$2")
        .bind(f.community.as_uuid())
        .bind(anchor.id.as_bytes().as_slice())
        .execute(&f.db.pool)
        .await
        .unwrap();
    let update = f.revision(d, "update", f.a, Some(&create), &f.peer, root());
    assert!(matches!(
        f.accept(&update, None).await,
        ArtifactOutcome::Accepted(_)
    ));
    let bad_delete = f.revision(d, "delete", f.a, Some(&update), &f.owner, vec![]);
    assert!(matches!(
        f.accept(&bad_delete, None).await,
        ArtifactOutcome::Rejected(_)
    ));
    let other = Fixture::new().await;
    assert!(!other
        .db
        .artifact_accepted(other.community, create.id.as_bytes())
        .await
        .unwrap());
    let same_id = other.revision(d, "create", other.a, None, &other.owner, vec![]);
    assert!(matches!(
        other.accept(&same_id, None).await,
        ArtifactOutcome::Accepted(_)
    ));
    assert_eq!(
        other
            .count(
                &other.owner,
                serde_json::json!({"artifact":"history","#d":[d]})
            )
            .await,
        1
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn concurrent_artifacts_on_ttl_channel_commit() {
    let f = Fixture::new().await;
    sqlx::query("UPDATE channels SET ttl_seconds=60 WHERE community_id=$1 AND id=$2")
        .bind(f.community.as_uuid())
        .bind(f.a)
        .execute(&f.db.pool)
        .await
        .unwrap();
    let a = f.revision(Uuid::new_v4(), "create", f.a, None, &f.owner, vec![]);
    let b = f.revision(Uuid::new_v4(), "create", f.a, None, &f.peer, vec![]);
    let (a, b) = tokio::join!(f.accept(&a, None), f.accept(&b, None));
    assert!(matches!(a, ArtifactOutcome::Accepted(_)));
    assert!(matches!(b, ArtifactOutcome::Accepted(_)));
    let live: bool = sqlx::query_scalar(
        "SELECT ttl_deadline>now() FROM channels WHERE community_id=$1 AND id=$2",
    )
    .bind(f.community.as_uuid())
    .bind(f.a)
    .fetch_one(&f.db.pool)
    .await
    .unwrap();
    assert!(live);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn quiescing_community_rejects_artifact_at_admission_before_coordinate_lock() {
    let f = Fixture::new().await;
    let d = Uuid::new_v4();
    let create = f.revision(d, "create", f.a, None, &f.owner, vec![]);
    let env = artifact::validate(&create).unwrap();
    crate::test_support::quiesce_community_for_tests(&f.db.pool, f.community).await;

    // Hold the artifact coordinate lock. Admission must reject before
    // `accept_artifact` reaches it, so the write cannot queue behind it.
    let mut holder = f.db.pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("artifact:{}:{}", f.community.as_uuid(), env.id))
        .execute(&mut *holder)
        .await
        .unwrap();
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        f.db.accept_artifact(f.community, &create, &env, None, &f.relay),
    )
    .await
    .expect("admission must reject before waiting on the coordinate lock")
    .expect_err("a quiescing community must reject artifact writes");
    holder.rollback().await.unwrap();
    assert!(
        crate::test_support::is_admission_rejection(&error),
        "expected entry admission rejection, got: {error:#}"
    );

    let persisted: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM events WHERE community_id=$1 AND id=$2), \
                (SELECT count(*) FROM artifact_heads WHERE community_id=$1 AND artifact_id=$3)",
    )
    .bind(f.community.as_uuid())
    .bind(create.id.as_bytes().as_slice())
    .bind(env.id)
    .fetch_one(&f.db.pool)
    .await
    .unwrap();
    assert_eq!(
        persisted,
        (0, 0),
        "a rejected artifact must persist nothing"
    );
}

const DIGEST: &str = "2222222222222222222222222222222222222222222222222222222222222222";

/// A `synaxis.html-review` revision whose content carries `synaxis`/`digest`,
/// signed by the fixture owner (the stand-in for the Synaxis adapter).
fn review(
    f: &Fixture,
    d: Uuid,
    home: Uuid,
    prev: Option<&Event>,
    synaxis: &str,
    digest: &str,
) -> Event {
    let op = if prev.is_some() { "update" } else { "create" };
    review_op(&f.owner, op, d, home, prev, synaxis, digest)
}

/// A `synaxis.html-review` revision with an explicit signer and operation.
fn review_op(
    key: &Keys,
    op: &str,
    d: Uuid,
    home: Uuid,
    prev: Option<&Event>,
    synaxis: &str,
    digest: &str,
) -> Event {
    let mut tags = vec![
        vec!["ar".to_string(), "1".into()],
        vec!["d".into(), d.to_string()],
        vec!["h".into(), home.to_string()],
        vec!["type".into(), "synaxis.html-review".into()],
        vec!["op".into(), op.into()],
    ];
    if op != "delete" {
        tags.push(vec!["title".into(), "Review".into()]);
    }
    if let Some(prev) = prev {
        tags.push(vec!["prev".into(), prev.id.to_hex()]);
    }
    let content = if op == "delete" {
        String::new()
    } else {
        serde_json::json!({
            "schema": "synaxis.html-review/v1",
            "artifact": { "id": synaxis, "payload_digest": digest },
            "presentation": { "blob_sha256": digest },
        })
        .to_string()
    };
    EventBuilder::new(Kind::Custom(45010), content)
        .tags(tags.into_iter().map(|t| Tag::parse(t).unwrap()))
        .sign_with_keys(key)
        .unwrap()
}

/// A feedback create (or `op` follow-up) against `target`, claiming `synaxis`/`digest`.
fn feedback(
    f: &Fixture,
    d: Uuid,
    home: Uuid,
    target: &Event,
    review_d: Uuid,
    synaxis: &str,
    digest: &str,
    key: &Keys,
) -> Event {
    let content = serde_json::json!({
        "schema": "synaxis.artifact-feedback/v1",
        "reviewed": {
            "buzz_artifact_id": review_d.to_string(),
            "buzz_revision_event_id": target.id.to_hex(),
            "synaxis_artifact_id": synaxis,
            "payload_digest": digest,
        },
        "target": { "review_id": "checkout.primary-action", "title": "Primary" },
        "request": "Move this above the summary.",
    });
    let _ = f;
    let tags = vec![
        vec!["ar".to_string(), "1".into()],
        vec!["d".into(), d.to_string()],
        vec!["h".into(), home.to_string()],
        vec!["type".into(), "synaxis.artifact-feedback".into()],
        vec!["title".into(), "Feedback".into()],
        vec!["op".into(), "create".into()],
        vec!["target".into(), "checkout.primary-action".into()],
        vec!["target_revision".into(), target.id.to_hex()],
        vec!["synaxis_artifact".into(), synaxis.into()],
        vec!["payload".into(), digest.into()],
    ];
    EventBuilder::new(Kind::Custom(45010), content.to_string())
        .tags(tags.into_iter().map(|t| Tag::parse(t).unwrap()))
        .sign_with_keys(key)
        .unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn feedback_is_accepted_only_against_the_current_review_head() {
    let f = Fixture::new().await;
    let review_d = Uuid::new_v4();
    let r1 = review(&f, review_d, f.a, None, "01SYN", DIGEST);
    assert!(matches!(
        f.accept(&r1, None).await,
        ArtifactOutcome::Accepted(_)
    ));

    // The current head takes feedback from any channel writer.
    let ok = feedback(
        &f,
        Uuid::new_v4(),
        f.a,
        &r1,
        review_d,
        "01SYN",
        DIGEST,
        &f.peer,
    );
    assert!(matches!(
        f.accept(&ok, None).await,
        ArtifactOutcome::Accepted(_)
    ));
    // An already accepted feedback event stays idempotent even after the head advances.
    let r2 = review(&f, review_d, f.a, Some(&r1), "01SYN", DIGEST);
    assert!(matches!(
        f.accept(&r2, None).await,
        ArtifactOutcome::Accepted(_)
    ));
    assert!(matches!(
        f.accept(&ok, None).await,
        ArtifactOutcome::Duplicate
    ));

    // A stale revision can no longer be commented on; the new head can.
    let stale = feedback(
        &f,
        Uuid::new_v4(),
        f.a,
        &r1,
        review_d,
        "01SYN",
        DIGEST,
        &f.peer,
    );
    assert!(matches!(
        f.accept(&stale, None).await,
        ArtifactOutcome::Conflict(_)
    ));
    let fresh = feedback(
        &f,
        Uuid::new_v4(),
        f.a,
        &r2,
        review_d,
        "01SYN",
        DIGEST,
        &f.peer,
    );
    assert!(matches!(
        f.accept(&fresh, None).await,
        ArtifactOutcome::Accepted(_)
    ));

    // A deleted review has no head to comment on, even by its exact revision ID.
    // Review deletion is refused through admission (see the lifecycle test), so a
    // soft-deleted head is simulated directly.
    sqlx::query("UPDATE artifact_heads SET deleted=true WHERE community_id=$1 AND artifact_id=$2")
        .bind(f.community.as_uuid())
        .bind(review_d)
        .execute(&f.db.pool)
        .await
        .unwrap();
    let on_deleted_revision = feedback(
        &f,
        Uuid::new_v4(),
        f.a,
        &r2,
        review_d,
        "01SYN",
        DIGEST,
        &f.peer,
    );
    assert!(matches!(
        f.accept(&on_deleted_revision, None).await,
        ArtifactOutcome::Conflict(_)
    ));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn feedback_must_bind_the_reviewed_artifact_digest_and_channel() {
    let f = Fixture::new().await;
    let (review_d, other_d) = (Uuid::new_v4(), Uuid::new_v4());
    let r = review(&f, review_d, f.a, None, "01SYN", DIGEST);
    let other = review(&f, other_d, f.a, None, "01OTHER", &"3".repeat(64));
    for e in [&r, &other] {
        assert!(matches!(
            f.accept(e, None).await,
            ArtifactOutcome::Accepted(_)
        ));
    }

    // Wrong Synaxis artifact or digest for a real head: a binding violation.
    for (synaxis, digest) in [("01OTHER", DIGEST), ("01SYN", "4".repeat(64).as_str())] {
        let forged = feedback(
            &f,
            Uuid::new_v4(),
            f.a,
            &r,
            review_d,
            synaxis,
            digest,
            &f.peer,
        );
        assert!(
            matches!(f.accept(&forged, None).await, ArtifactOutcome::Rejected(_)),
            "{synaxis}"
        );
    }
    // Another artifact's head named under this artifact's UUID is not this head.
    let cross = feedback(
        &f,
        Uuid::new_v4(),
        f.a,
        &other,
        review_d,
        "01OTHER",
        &"3".repeat(64),
        &f.peer,
    );
    assert!(matches!(
        f.accept(&cross, None).await,
        ArtifactOutcome::Conflict(_)
    ));
    // A reviewed artifact that does not exist leaks nothing and is refused.
    let missing = feedback(
        &f,
        Uuid::new_v4(),
        f.a,
        &r,
        Uuid::new_v4(),
        "01SYN",
        DIGEST,
        &f.peer,
    );
    assert!(matches!(
        f.accept(&missing, None).await,
        ArtifactOutcome::Conflict(_)
    ));
    // Feedback written into a different channel than the review's home.
    let cross_channel = feedback(
        &f,
        Uuid::new_v4(),
        f.b,
        &r,
        review_d,
        "01SYN",
        DIGEST,
        &f.owner,
    );
    assert!(matches!(
        f.accept(&cross_channel, None).await,
        ArtifactOutcome::Conflict(_)
    ));
    // Feedback cannot name something that is not a review at all.
    let task_d = Uuid::new_v4();
    let task = f.revision(task_d, "create", f.a, None, &f.owner, vec![]);
    assert!(matches!(
        f.accept(&task, None).await,
        ArtifactOutcome::Accepted(_)
    ));
    let not_review = feedback(
        &f,
        Uuid::new_v4(),
        f.a,
        &task,
        task_d,
        "01SYN",
        DIGEST,
        &f.peer,
    );
    assert!(matches!(
        f.accept(&not_review, None).await,
        ArtifactOutcome::Conflict(_)
    ));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn accepted_feedback_is_immutable() {
    let f = Fixture::new().await;
    let review_d = Uuid::new_v4();
    let r = review(&f, review_d, f.a, None, "01SYN", DIGEST);
    assert!(matches!(
        f.accept(&r, None).await,
        ArtifactOutcome::Accepted(_)
    ));
    let fb_d = Uuid::new_v4();
    let fb = feedback(&f, fb_d, f.a, &r, review_d, "01SYN", DIGEST, &f.peer);
    assert!(matches!(
        f.accept(&fb, None).await,
        ArtifactOutcome::Accepted(_)
    ));

    // Any channel writer holds update/delete rights over a generic artifact; for
    // feedback every follow-up operation is refused, so it cannot be suppressed.
    for op in ["update", "delete"] {
        let mut tags: Vec<Tag> = fb
            .tags
            .iter()
            .filter(|t| {
                op != "delete" || ["ar", "d", "h", "type"].contains(&t.as_slice()[0].as_str())
            })
            .map(|t| match t.as_slice()[0].as_str() {
                "op" => Tag::parse(["op", op]).unwrap(),
                _ => t.clone(),
            })
            .collect();
        if op == "delete" {
            tags.push(Tag::parse(["op", "delete"]).unwrap());
        }
        tags.push(Tag::parse(["prev", &fb.id.to_hex()]).unwrap());
        let content = if op == "delete" {
            String::new()
        } else {
            fb.content.clone()
        };
        let attempt = EventBuilder::new(Kind::Custom(45010), content)
            .tags(tags)
            .sign_with_keys(&f.owner)
            .unwrap();
        assert!(
            matches!(f.accept(&attempt, None).await, ArtifactOutcome::Rejected(_)),
            "{op}"
        );
    }
    // The original create is still the artifact's current head.
    let current = f
        .query(
            &f.owner,
            serde_json::json!({"artifact":"current","#d":[fb_d]}),
            false,
        )
        .await
        .0;
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].event.id, fb.id);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn feedback_and_a_concurrent_head_advance_serialize() {
    for _ in 0..8 {
        let f = Fixture::new().await;
        let review_d = Uuid::new_v4();
        let r1 = review(&f, review_d, f.a, None, "01SYN", DIGEST);
        assert!(matches!(
            f.accept(&r1, None).await,
            ArtifactOutcome::Accepted(_)
        ));
        let fb = feedback(
            &f,
            Uuid::new_v4(),
            f.a,
            &r1,
            review_d,
            "01SYN",
            DIGEST,
            &f.peer,
        );
        let r2 = review(&f, review_d, f.a, Some(&r1), "01SYN", DIGEST);
        let (fb_outcome, r2_outcome) = tokio::join!(f.accept(&fb, None), f.accept(&r2, None));
        // The head advance always wins or waits; the feedback is either stored
        // against r1 before the advance, or refused as stale. Never silently
        // stored against a head that had already moved.
        assert!(
            matches!(r2_outcome, ArtifactOutcome::Accepted(_)),
            "{r2_outcome:?}"
        );
        match fb_outcome {
            ArtifactOutcome::Accepted(_) => {
                let stored = f.count(&f.owner, serde_json::json!({"artifact":"current","#target_revision":[r1.id.to_hex()]})).await;
                assert_eq!(stored, 1);
            }
            ArtifactOutcome::Conflict(_) => {}
            other => panic!("unexpected feedback outcome: {other:?}"),
        }
        // Once the head has moved, the same stale feedback is always refused.
        let again = feedback(
            &f,
            Uuid::new_v4(),
            f.a,
            &r1,
            review_d,
            "01SYN",
            DIGEST,
            &f.peer,
        );
        assert!(matches!(
            f.accept(&again, None).await,
            ArtifactOutcome::Conflict(_)
        ));
    }
}

/// Id of the current, live head for `d`, or `None`.
async fn current_head(f: &Fixture, d: Uuid) -> Option<nostr::EventId> {
    f.query(
        &f.owner,
        serde_json::json!({"artifact":"current","#d":[d]}),
        false,
    )
    .await
    .0
    .first()
    .map(|stored| stored.event.id)
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn review_head_belongs_to_its_signer_and_only_accepts_same_home_updates() {
    let f = Fixture::new().await;
    let review_d = Uuid::new_v4();
    let r1 = review(&f, review_d, f.a, None, "01SYN", DIGEST);
    assert!(matches!(
        f.accept(&r1, None).await,
        ArtifactOutcome::Accepted(_)
    ));

    // Another channel writer holds the exact current `prev`, yet cannot update,
    // delete, move, or restore the adapter's review.
    for (op, home) in [
        ("update", f.a),
        ("delete", f.a),
        ("move", f.b),
        ("restore", f.a),
    ] {
        let foreign = review_op(&f.peer, op, review_d, home, Some(&r1), "01SYN", DIGEST);
        let outcome = f.accept(&foreign, Some(f.a)).await;
        assert!(
            matches!(outcome, ArtifactOutcome::Rejected(_)),
            "foreign {op}: {outcome:?}"
        );
    }
    // Not even the original signer has a delete/move/restore lifecycle, and an
    // update cannot change home.
    for (op, home) in [
        ("delete", f.a),
        ("move", f.b),
        ("restore", f.a),
        ("update", f.b),
    ] {
        let own = review_op(&f.owner, op, review_d, home, Some(&r1), "01SYN", DIGEST);
        let outcome = f.accept(&own, Some(f.a)).await;
        assert!(
            matches!(outcome, ArtifactOutcome::Rejected(_)),
            "signer {op}: {outcome:?}"
        );
    }
    // Every refusal left the head, its home, and its feedback target untouched.
    assert_eq!(current_head(&f, review_d).await, Some(r1.id));
    let fb = feedback(
        &f,
        Uuid::new_v4(),
        f.a,
        &r1,
        review_d,
        "01SYN",
        DIGEST,
        &f.peer,
    );
    assert!(matches!(
        f.accept(&fb, None).await,
        ArtifactOutcome::Accepted(_)
    ));

    // The original signer advances with the exact `prev`; a different `prev`
    // from the same signer still loses the ordinary CAS.
    let r2 = review(&f, review_d, f.a, Some(&r1), "01SYN", DIGEST);
    assert!(matches!(
        f.accept(&r2, None).await,
        ArtifactOutcome::Accepted(_)
    ));
    let stale = review_op(
        &f.owner,
        "update",
        review_d,
        f.a,
        Some(&r1),
        "01OTHER",
        DIGEST,
    );
    assert!(matches!(
        f.accept(&stale, None).await,
        ArtifactOutcome::Conflict(_)
    ));

    // Ownership follows the current head, so a foreign update on it is refused too.
    let r3 = review(&f, review_d, f.a, Some(&r2), "01SYN", DIGEST);
    assert!(matches!(
        f.accept(&r3, None).await,
        ArtifactOutcome::Accepted(_)
    ));
    let foreign = review_op(&f.peer, "update", review_d, f.a, Some(&r3), "01SYN", DIGEST);
    assert!(matches!(
        f.accept(&foreign, None).await,
        ArtifactOutcome::Rejected(_)
    ));
    assert_eq!(current_head(&f, review_d).await, Some(r3.id));

    // Creation is unchanged: any channel writer may claim an unused identity,
    // and then holds it exclusively.
    let claimed_d = Uuid::new_v4();
    let claim = review_op(&f.peer, "create", claimed_d, f.a, None, "01PEER", DIGEST);
    assert!(matches!(
        f.accept(&claim, None).await,
        ArtifactOutcome::Accepted(_)
    ));
    let takeover = review_op(
        &f.owner,
        "update",
        claimed_d,
        f.a,
        Some(&claim),
        "01PEER",
        DIGEST,
    );
    assert!(matches!(
        f.accept(&takeover, None).await,
        ArtifactOutcome::Rejected(_)
    ));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn review_head_without_a_retained_signer_fails_closed() {
    let f = Fixture::new().await;
    let review_d = Uuid::new_v4();
    let r1 = review(&f, review_d, f.a, None, "01SYN", DIGEST);
    assert!(matches!(
        f.accept(&r1, None).await,
        ArtifactOutcome::Accepted(_)
    ));
    sqlx::query("DELETE FROM events WHERE community_id=$1 AND id=$2")
        .bind(f.community.as_uuid())
        .bind(r1.id.as_bytes().as_slice())
        .execute(&f.db.pool)
        .await
        .unwrap();
    let r2 = review(&f, review_d, f.a, Some(&r1), "01SYN", DIGEST);
    assert!(matches!(
        f.accept(&r2, None).await,
        ArtifactOutcome::Rejected(_)
    ));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn non_review_artifacts_keep_collaborative_lifecycle() {
    let f = Fixture::new().await;
    let d = Uuid::new_v4();
    let create = f.revision(d, "create", f.a, None, &f.owner, vec![]);
    assert!(matches!(
        f.accept(&create, None).await,
        ArtifactOutcome::Accepted(_)
    ));
    // A different channel writer may still update and then delete a generic artifact.
    let update = f.revision(d, "update", f.a, Some(&create), &f.peer, vec![]);
    assert!(matches!(
        f.accept(&update, None).await,
        ArtifactOutcome::Accepted(_)
    ));
    let delete = f.revision(d, "delete", f.a, Some(&update), &f.peer, vec![]);
    assert!(matches!(
        f.accept(&delete, None).await,
        ArtifactOutcome::Accepted(_)
    ));
}

/// An accepted feedback revision signed by `author`, plus the review head it
/// names, with the tag layout a review-feedback wake carries.
struct WakeScene {
    home: Uuid,
    author: nostr::PublicKey,
    review_d: Uuid,
    review: Event,
    feedback_d: Uuid,
    feedback: Event,
    agent: String,
    parent: String,
}

impl WakeScene {
    async fn new(f: &Fixture, author: &Keys, home: Uuid) -> Self {
        let review_d = Uuid::new_v4();
        let review_rev = review(f, review_d, home, None, "01SYN", DIGEST);
        assert!(matches!(
            f.accept(&review_rev, None).await,
            ArtifactOutcome::Accepted(_)
        ));
        let feedback_d = Uuid::new_v4();
        let feedback_rev = feedback(
            f,
            feedback_d,
            home,
            &review_rev,
            review_d,
            "01SYN",
            DIGEST,
            author,
        );
        assert!(matches!(
            f.accept(&feedback_rev, None).await,
            ArtifactOutcome::Accepted(_)
        ));
        Self {
            home,
            author: author.public_key(),
            review_d,
            review: review_rev,
            feedback_d,
            feedback: feedback_rev,
            agent: Keys::generate().public_key().to_hex(),
            parent: Keys::generate().public_key().to_hex(),
        }
    }

    fn tags(&self) -> Vec<Vec<String>> {
        vec![
            vec!["h".into(), self.home.to_string()],
            vec!["e".into(), self.parent.clone(), "".into(), "reply".into()],
            vec!["p".into(), self.agent.clone()],
            vec![
                "feedback".into(),
                self.feedback_d.to_string(),
                self.feedback.id.to_hex(),
            ],
            vec![
                "artifact".into(),
                self.review_d.to_string(),
                self.review.id.to_hex(),
            ],
        ]
    }

    /// A wake signed by `key`, stamped `offset` seconds after the current time:
    /// each distinct offset yields a distinct event ID for identical bindings.
    fn wake(&self, key: &Keys, offset: u64) -> Event {
        self.wake_with(key, offset, |_| {})
    }

    fn wake_with(&self, key: &Keys, offset: u64, tweak: impl Fn(&mut Vec<Vec<String>>)) -> Event {
        let mut tags = self.tags();
        tweak(&mut tags);
        EventBuilder::new(Kind::Custom(9), "Please address the feedback.")
            .tags(tags.into_iter().map(|t| Tag::parse(t).unwrap()))
            .custom_created_at(nostr::Timestamp::from(
                nostr::Timestamp::now().as_secs() + offset,
            ))
            .sign_with_keys(key)
            .unwrap()
    }
}

/// Replace element `at` of the first tag named `name`.
fn set_tag(tags: &mut [Vec<String>], name: &str, at: usize, value: String) {
    tags.iter_mut().find(|t| t[0] == name).unwrap()[at] = value;
}

async fn admit(f: &Fixture, channel: Uuid, event: &Event) -> FeedbackWakeOutcome {
    let wake = artifact::validate_feedback_wake(event).unwrap();
    f.db.accept_feedback_wake(f.community, event, channel, &wake, None)
        .await
        .unwrap()
}

/// `(stored kind-9 events, recorded wakes)` in the fixture community.
async fn wake_counts(f: &Fixture) -> (i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM events WHERE community_id=$1 AND kind=9), \
                (SELECT count(*) FROM artifact_feedback_wakes WHERE community_id=$1)",
    )
    .bind(f.community.as_uuid())
    .fetch_one(&f.db.pool)
    .await
    .unwrap()
}

/// Whether the ledger records `wake` as `author`'s wake of some feedback revision.
async fn wake_recorded(f: &Fixture, author: &[u8], wake: &Event) -> bool {
    f.db.feedback_wake_event_accepted(f.community, author, wake.id.as_bytes())
        .await
        .unwrap()
}

const BINDINGS_CONFLICT: &str =
    "a wake for this feedback revision already exists with different bindings";
const NO_FEEDBACK: &str = "wake names no accepted feedback revision of this author";

#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_valid_wake_is_recorded_once_and_retries_store_nothing() {
    let f = Fixture::new().await;
    let scene = WakeScene::new(&f, &f.peer, f.a).await;
    let w1 = scene.wake(&f.peer, 0);

    let FeedbackWakeOutcome::Inserted(stored) = admit(&f, f.a, &w1).await else {
        panic!("the first wake is admitted");
    };
    assert_eq!(stored.event.id, w1.id);
    assert_eq!(stored.channel_id, Some(f.a));
    assert_eq!(wake_counts(&f).await, (1, 1));

    // A lost ACK resends the identical event.
    assert!(matches!(
        admit(&f, f.a, &w1).await,
        FeedbackWakeOutcome::Duplicate
    ));
    // A retry after the freshness window refused the first attempt carries a
    // fresh timestamp, hence another event ID, with the same bindings.
    for offset in [1, 2] {
        let retry = scene.wake(&f.peer, offset);
        assert_ne!(retry.id, w1.id);
        assert!(matches!(
            admit(&f, f.a, &retry).await,
            FeedbackWakeOutcome::Duplicate
        ));
    }
    assert_eq!(wake_counts(&f).await, (1, 1));

    let recorded: Vec<u8> = sqlx::query_scalar(
        "SELECT wake_event_id FROM artifact_feedback_wakes WHERE community_id=$1 AND feedback_event_id=$2",
    )
    .bind(f.community.as_uuid())
    .bind(scene.feedback.id.as_bytes().as_slice())
    .fetch_one(&f.db.pool)
    .await
    .unwrap();
    assert_eq!(recorded, w1.id.as_bytes().to_vec());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_wake_with_other_bindings_for_a_recorded_feedback_is_refused() {
    let f = Fixture::new().await;
    let scene = WakeScene::new(&f, &f.peer, f.a).await;
    assert!(matches!(
        admit(&f, f.a, &scene.wake(&f.peer, 0)).await,
        FeedbackWakeOutcome::Inserted(_)
    ));

    let other_id = Keys::generate().public_key().to_hex();
    let variants = [
        (
            "agent",
            scene.wake_with(&f.peer, 1, |t| set_tag(t, "p", 1, other_id.clone())),
        ),
        (
            "parent",
            scene.wake_with(&f.peer, 2, |t| set_tag(t, "e", 1, other_id.clone())),
        ),
        (
            "thread root",
            scene.wake_with(&f.peer, 3, |t| {
                t.push(vec!["e".into(), other_id.clone(), "".into(), "root".into()])
            }),
        ),
        (
            "reviewed revision",
            scene.wake_with(&f.peer, 4, |t| set_tag(t, "artifact", 2, other_id.clone())),
        ),
        (
            "reviewed artifact",
            scene.wake_with(&f.peer, 5, |t| {
                set_tag(t, "artifact", 1, Uuid::new_v4().to_string())
            }),
        ),
    ];
    for (what, variant) in variants {
        assert!(
            matches!(
                admit(&f, f.a, &variant).await,
                FeedbackWakeOutcome::Rejected(BINDINGS_CONFLICT)
            ),
            "{what}"
        );
    }
    // The same wake aimed at another channel is a different binding too.
    let moved = EventBuilder::new(Kind::Custom(9), "Please address the feedback.")
        .tags(scene.tags().into_iter().map(|mut t| {
            if t[0] == "h" {
                t[1] = f.b.to_string();
            }
            Tag::parse(t).unwrap()
        }))
        .sign_with_keys(&f.peer)
        .unwrap();
    assert!(matches!(
        admit(&f, f.b, &moved).await,
        FeedbackWakeOutcome::Rejected(BINDINGS_CONFLICT)
    ));
    assert_eq!(wake_counts(&f).await, (1, 1));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_wake_must_name_an_accepted_feedback_revision_of_its_author() {
    let f = Fixture::new().await;
    let scene = WakeScene::new(&f, &f.peer, f.a).await;
    let other_id = Keys::generate().public_key().to_hex();

    // Nobody wakes for someone else's feedback.
    let by_owner = scene.wake(&f.owner, 0);
    assert!(matches!(
        admit(&f, f.a, &by_owner).await,
        FeedbackWakeOutcome::Rejected(NO_FEEDBACK)
    ));
    // Neither does the author name a revision that does not exist, an
    // artifact the feedback does not belong to, or a review in its place.
    let unknown = scene.wake_with(&f.peer, 1, |t| set_tag(t, "feedback", 2, other_id.clone()));
    let wrong_artifact = scene.wake_with(&f.peer, 2, |t| {
        set_tag(t, "feedback", 1, Uuid::new_v4().to_string())
    });
    let a_review = scene.wake_with(&f.owner, 3, |t| {
        set_tag(t, "feedback", 1, scene.review_d.to_string());
        set_tag(t, "feedback", 2, scene.review.id.to_hex());
    });
    // The signed feedback content names its reviewed revision and artifact.
    let wrong_reviewed_revision =
        scene.wake_with(&f.peer, 4, |t| set_tag(t, "artifact", 2, other_id.clone()));
    let wrong_reviewed_artifact = scene.wake_with(&f.peer, 5, |t| {
        set_tag(t, "artifact", 1, Uuid::new_v4().to_string())
    });
    for (what, wake) in [
        ("unknown revision", unknown),
        ("another feedback artifact", wrong_artifact),
        ("a review revision", a_review),
        ("another reviewed revision", wrong_reviewed_revision),
        ("another reviewed artifact", wrong_reviewed_artifact),
    ] {
        assert!(
            matches!(
                admit(&f, f.a, &wake).await,
                FeedbackWakeOutcome::Rejected(NO_FEEDBACK)
            ),
            "{what}"
        );
    }
    // The feedback lives in channel a; a wake cannot move it to channel b.
    let in_b = EventBuilder::new(Kind::Custom(9), "Please address the feedback.")
        .tags(scene.tags().into_iter().map(|mut t| {
            if t[0] == "h" {
                t[1] = f.b.to_string();
            }
            Tag::parse(t).unwrap()
        }))
        .sign_with_keys(&f.peer)
        .unwrap();
    assert!(matches!(
        admit(&f, f.b, &in_b).await,
        FeedbackWakeOutcome::Rejected(NO_FEEDBACK)
    ));
    assert_eq!(wake_counts(&f).await, (0, 0));

    // Redacted feedback can no longer be woken for.
    assert!(f
        .db
        .soft_delete_event_and_update_thread(f.community, scene.feedback.id.as_bytes(), None, None)
        .await
        .unwrap());
    assert!(matches!(
        admit(&f, f.a, &scene.wake(&f.peer, 6)).await,
        FeedbackWakeOutcome::Rejected(NO_FEEDBACK)
    ));
    assert_eq!(wake_counts(&f).await, (0, 0));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn concurrent_wakes_for_one_feedback_store_exactly_one_event() {
    let f = Fixture::new().await;
    let scene = WakeScene::new(&f, &f.peer, f.a).await;
    let a = scene.wake(&f.peer, 0);
    let b = scene.wake(&f.peer, 1);
    assert_ne!(a.id, b.id);

    let (ra, rb) = tokio::join!(admit(&f, f.a, &a), admit(&f, f.a, &b));
    match (&ra, &rb) {
        (FeedbackWakeOutcome::Inserted(_), FeedbackWakeOutcome::Duplicate)
        | (FeedbackWakeOutcome::Duplicate, FeedbackWakeOutcome::Inserted(_)) => {}
        other => panic!("exactly one claimant is admitted: {other:?}"),
    }
    assert_eq!(wake_counts(&f).await, (1, 1));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn the_wake_ledger_recognises_only_the_recorded_wake_of_its_author() {
    let f = Fixture::new().await;
    let scene = WakeScene::new(&f, &f.peer, f.a).await;
    let author = scene.author.to_bytes();
    let w1 = scene.wake(&f.peer, 0);
    let retry = scene.wake(&f.peer, 1);

    assert!(!wake_recorded(&f, &author, &w1).await);
    assert!(matches!(
        admit(&f, f.a, &w1).await,
        FeedbackWakeOutcome::Inserted(_)
    ));
    assert!(wake_recorded(&f, &author, &w1).await);
    // Another signer, and a duplicate-acknowledged retry, are not the record.
    assert!(!wake_recorded(&f, &f.owner.public_key().to_bytes(), &w1).await);
    assert!(matches!(
        admit(&f, f.a, &retry).await,
        FeedbackWakeOutcome::Duplicate
    ));
    assert!(!wake_recorded(&f, &author, &retry).await);
    let elsewhere = CommunityId::from_uuid(Uuid::new_v4());
    assert!(!f
        .db
        .feedback_wake_event_accepted(elsewhere, &author, w1.id.as_bytes())
        .await
        .unwrap());

    // The record outlives the wake event itself: once retention removes it,
    // a resend is still acknowledged and is never minted again.
    sqlx::query("DELETE FROM events WHERE community_id=$1 AND id=$2")
        .bind(f.community.as_uuid())
        .bind(w1.id.as_bytes().as_slice())
        .execute(&f.db.pool)
        .await
        .unwrap();
    assert!(wake_recorded(&f, &author, &w1).await);
    assert!(matches!(
        admit(&f, f.a, &w1).await,
        FeedbackWakeOutcome::Duplicate
    ));
    assert!(matches!(
        admit(&f, f.a, &retry).await,
        FeedbackWakeOutcome::Duplicate
    ));
    assert_eq!(wake_counts(&f).await, (0, 1));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_replayed_stored_event_leaves_no_wake_claim() {
    let f = Fixture::new().await;
    let scene = WakeScene::new(&f, &f.peer, f.a).await;
    let w1 = scene.wake(&f.peer, 0);
    // The wake event is already stored (by any other path) without a claim.
    f.db.insert_event_with_thread_metadata(f.community, &w1, Some(f.a), None)
        .await
        .unwrap();
    assert!(matches!(
        admit(&f, f.a, &w1).await,
        FeedbackWakeOutcome::Duplicate
    ));
    // Claim and event are atomic: no claim was left behind for an event the
    // call did not add.
    assert_eq!(wake_counts(&f).await, (1, 0));
}
