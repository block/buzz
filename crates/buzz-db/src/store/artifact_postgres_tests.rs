//! PostgreSQL regression tests for the production artifact transaction/query seams.
use super::artifact::ArtifactOutcome;
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
        db.maintain_partitions(
            1,
            crate::partition::PartitionMaintenancePolicy {
                create_enabled: true,
                advance_enabled: false,
            },
        )
        .await
        .unwrap();
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
