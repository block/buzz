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
        Self::on(Db::from_pool(pool)).await
    }
    /// The fixture on `db`, which is already migrated.
    async fn on(db: Db) -> Self {
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

/// An artifact in a system channel is read only by a query whose `#h` names
/// that channel (NIP-CL), even when the channel is open.
/// Mutation: drop the `channel_type` condition from `query_artifacts` → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn system_channel_artifacts_are_read_only_by_name() {
    let f = Fixture::new().await;
    let system = Uuid::new_v4();
    sqlx::query("INSERT INTO channels(community_id,id,name,visibility,channel_type,created_by) VALUES($1,$2,$3,'open','system',$4)")
        .bind(f.community.as_uuid())
        .bind(system)
        .bind(system.to_string())
        .bind(f.owner.public_key().to_bytes().as_slice())
        .execute(&f.db.pool)
        .await
        .unwrap();
    for home in [f.a, system] {
        let create = f.revision(Uuid::new_v4(), "create", home, None, &f.owner, vec![]);
        assert!(matches!(
            f.accept(&create, None).await,
            ArtifactOutcome::Accepted(_)
        ));
    }
    for reader in [&f.owner, &f.peer] {
        let (events, _) = f
            .query(reader, serde_json::json!({"artifact":"current"}), false)
            .await;
        let homes: Vec<_> = events.iter().map(|e| e.channel_id).collect();
        assert_eq!(homes, [Some(f.a)], "no #h");
        assert_eq!(
            f.count(reader, serde_json::json!({"artifact":"current"}))
                .await,
            1
        );
        let named = serde_json::json!({"artifact":"current","#h":[system.to_string()]});
        let (events, _) = f.query(reader, named.clone(), false).await;
        let homes: Vec<_> = events.iter().map(|e| e.channel_id).collect();
        assert_eq!(homes, [Some(system)], "#h names the system channel");
        assert_eq!(f.count(reader, named).await, 1);
    }
}

/// An artifact in an unlisted open channel is read by a non-member only
/// through a query whose `#h` names that channel; a member reads it as usual.
/// Mutation: drop the unlisted condition from `query_artifacts` → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn unlisted_channel_artifacts_are_read_by_name_for_non_members() {
    let f = Fixture::new().await;
    let unlisted = Uuid::new_v4();
    sqlx::query("INSERT INTO channels(community_id,id,name,visibility,channel_type,created_by,unlisted) VALUES($1,$2,$3,'open','stream',$4,TRUE)")
        .bind(f.community.as_uuid())
        .bind(unlisted)
        .bind(unlisted.to_string())
        .bind(f.owner.public_key().to_bytes().as_slice())
        .execute(&f.db.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO channel_members(community_id,channel_id,pubkey,role) VALUES($1,$2,$3,'owner')",
    )
    .bind(f.community.as_uuid())
    .bind(unlisted)
    .bind(f.owner.public_key().to_bytes().as_slice())
    .execute(&f.db.pool)
    .await
    .unwrap();
    for home in [f.a, unlisted] {
        let create = f.revision(Uuid::new_v4(), "create", home, None, &f.owner, vec![]);
        assert!(matches!(
            f.accept(&create, None).await,
            ArtifactOutcome::Accepted(_)
        ));
    }
    let unnamed = serde_json::json!({"artifact":"current"});
    let named = serde_json::json!({"artifact":"current","#h":[unlisted.to_string()]});
    let homes = |events: Vec<buzz_core::StoredEvent>| {
        let mut homes: Vec<_> = events.iter().map(|e| e.channel_id).collect();
        homes.sort();
        homes
    };

    // The member reads both without naming either.
    let (events, _) = f.query(&f.owner, unnamed.clone(), false).await;
    let mut both = vec![Some(f.a), Some(unlisted)];
    both.sort();
    assert_eq!(homes(events), both, "member without #h");

    // A non-member reads the unlisted one only by naming it.
    let (events, _) = f.query(&f.peer, unnamed.clone(), false).await;
    assert_eq!(homes(events), [Some(f.a)], "non-member without #h");
    assert_eq!(f.count(&f.peer, unnamed).await, 1);
    let (events, _) = f.query(&f.peer, named.clone(), false).await;
    assert_eq!(homes(events), [Some(unlisted)], "non-member #h names it");
    assert_eq!(f.count(&f.peer, named).await, 1);
}

/// An artifact in a channel of a type added later is read only by a query
/// whose `#h` names that channel, like a system channel (NIP-CL).
/// Mutation: put back `c.channel_type <> 'system'` in `query_artifacts` → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn artifacts_of_a_later_channel_type_are_read_only_by_name() {
    use super::legacy_channel_types_postgres_tests::{scratch_db_with_future_type, FUTURE_TYPE};
    let (db, _scratch) = scratch_db_with_future_type().await;
    let f = Fixture::on(db).await;
    let later = Uuid::new_v4();
    sqlx::query("INSERT INTO channels(community_id,id,name,visibility,channel_type,created_by) VALUES($1,$2,$3,'open',$4::channel_type,$5)")
        .bind(f.community.as_uuid())
        .bind(later)
        .bind(later.to_string())
        .bind(FUTURE_TYPE)
        .bind(f.owner.public_key().to_bytes().as_slice())
        .execute(&f.db.pool)
        .await
        .unwrap();
    let mut ds = Vec::new();
    for home in [f.a, later] {
        let d = Uuid::new_v4();
        let create = f.revision(d, "create", home, None, &f.owner, vec![]);
        assert!(matches!(
            f.accept(&create, None).await,
            ArtifactOutcome::Accepted(_)
        ));
        let update = f.revision(d, "update", home, Some(&create), &f.owner, vec![]);
        assert!(matches!(
            f.accept(&update, None).await,
            ArtifactOutcome::Accepted(_)
        ));
        ds.push(d.to_string());
    }
    // History needs `#d`; both queries name both artifacts.
    for (view, per_channel) in [("current", 1), ("history", 2)] {
        let unnamed = serde_json::json!({ "artifact": view, "#d": ds });
        let (events, _) = f.query(&f.owner, unnamed.clone(), false).await;
        let homes: Vec<_> = events.iter().map(|e| e.channel_id).collect();
        assert_eq!(homes, vec![Some(f.a); per_channel], "{view} without #h");
        assert_eq!(f.count(&f.owner, unnamed).await, per_channel as i64);

        let named = serde_json::json!({ "artifact": view, "#d": ds, "#h": [later.to_string()] });
        let (events, _) = f.query(&f.owner, named.clone(), false).await;
        let homes: Vec<_> = events.iter().map(|e| e.channel_id).collect();
        assert_eq!(
            homes,
            vec![Some(later); per_channel],
            "{view} #h names the channel"
        );
        assert_eq!(f.count(&f.owner, named).await, per_channel as i64);
    }
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

/// A non-create revision whose head lives outside the channel the caller
/// authorized answers exactly like a missing artifact, before the revision,
/// type, deleted and root checks could describe the head.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn head_outside_authorized_channel_matches_missing_head() {
    let f = Fixture::new().await;
    let d = Uuid::new_v4();
    let create = f.revision(d, "create", f.a, None, &f.owner, vec![]);
    assert!(matches!(
        f.accept(&create, None).await,
        ArtifactOutcome::Accepted(_)
    ));
    let gone = Uuid::new_v4();
    let deleted_d = Uuid::new_v4();
    let deleted_create = f.revision(deleted_d, "create", f.a, None, &f.owner, vec![]);
    assert!(matches!(
        f.accept(&deleted_create, None).await,
        ArtifactOutcome::Accepted(_)
    ));
    let deleted = f.revision(
        deleted_d,
        "delete",
        f.a,
        Some(&deleted_create),
        &f.owner,
        vec![],
    );
    assert!(matches!(
        f.accept(&deleted, None).await,
        ArtifactOutcome::Accepted(_)
    ));
    let retyped = {
        let mut tags: Vec<Vec<String>> = vec![
            vec!["ar".into(), "1".into()],
            vec!["d".into(), d.to_string()],
            vec!["h".into(), f.b.to_string()],
            vec!["type".into(), "buzz.note".into()],
            vec!["op".into(), "update".into()],
            vec!["title".into(), "Test".into()],
        ];
        tags.push(vec!["prev".into(), create.id.to_hex()]);
        EventBuilder::new(Kind::Custom(45010), "secret")
            .tags(tags.into_iter().map(|t| Tag::parse(t).unwrap()))
            .sign_with_keys(&f.peer)
            .unwrap()
    };
    let cases = [
        (
            "missing",
            f.revision(gone, "update", f.b, Some(&create), &f.peer, vec![]),
            None,
        ),
        (
            "current prev",
            f.revision(d, "update", f.b, Some(&create), &f.peer, vec![]),
            None,
        ),
        (
            "stale prev",
            f.revision(d, "update", f.b, Some(&deleted), &f.peer, vec![]),
            None,
        ),
        ("other type", retyped, None),
        (
            "delete",
            f.revision(d, "delete", f.b, Some(&create), &f.peer, vec![]),
            None,
        ),
        (
            "restore deleted",
            f.revision(deleted_d, "restore", f.b, Some(&deleted), &f.peer, vec![]),
            None,
        ),
        (
            "update deleted",
            f.revision(deleted_d, "update", f.b, Some(&deleted), &f.peer, vec![]),
            None,
        ),
        (
            "move from unauthorized source",
            f.revision(d, "move", f.b, Some(&create), &f.peer, vec![]),
            Some(f.b),
        ),
    ];
    for (case, event, source) in cases {
        match f.accept(&event, source).await {
            ArtifactOutcome::Conflict(reason) => {
                assert_eq!(reason, super::artifact::HEAD_UNAVAILABLE, "{case}")
            }
            other => panic!("{case}: {other:?}"),
        }
    }
    // Positive control: the same revision homed where the head lives.
    let update = f.revision(d, "update", f.a, Some(&create), &f.peer, vec![]);
    assert!(matches!(
        f.accept(&update, None).await,
        ArtifactOutcome::Accepted(_)
    ));
}

/// Sync's complete-set read: keyset pages by `d` share one snapshot, so a
/// change committed after it opens is in no page. Mutation: drop the
/// `REPEATABLE READ` statement → the late create appears in a later page →
/// RED. Mutation: drop the `a.artifact_id>` keyset bound → pages repeat → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn sync_snapshot_pages_are_one_cut_and_head_includes_deletions() {
    let f = Fixture::new().await;
    let mut ids: Vec<Uuid> = (0..5).map(|_| Uuid::new_v4()).collect();
    ids.sort();
    let mut heads = vec![];
    for d in &ids {
        let create = f.revision(*d, "create", f.a, None, &f.owner, vec![]);
        assert!(matches!(
            f.accept(&create, None).await,
            ArtifactOutcome::Accepted(_)
        ));
        heads.push(create);
    }
    // A deleted artifact leaves the set but stays visible to the head view.
    let deleted = f.revision(ids[4], "delete", f.a, Some(&heads[4]), &f.owner, vec![]);
    assert!(matches!(
        f.accept(&deleted, None).await,
        ArtifactOutcome::Accepted(_)
    ));
    let reader = f.owner.public_key();
    let mut snapshot =
        f.db.open_artifact_snapshot(f.community, reader.as_bytes(), &[f.a])
            .await
            .unwrap()
            .unwrap();
    let first = snapshot.next_page(&[], 2).await.unwrap();
    // Committed after the snapshot opened, and its `d` sorts after the first
    // page, so only a fresh snapshot per page would return it.
    let late = loop {
        let d = Uuid::new_v4();
        if d > ids[1] {
            break d;
        }
    };
    let late_create = f.revision(late, "create", f.a, None, &f.owner, vec![]);
    assert!(matches!(
        f.accept(&late_create, None).await,
        ArtifactOutcome::Accepted(_)
    ));
    let mut seen: Vec<_> = first.iter().map(|e| e.event.id).collect();
    loop {
        let page = snapshot.next_page(&[], 2).await.unwrap();
        if page.is_empty() {
            break;
        }
        seen.extend(page.iter().map(|e| e.event.id));
    }
    snapshot.finish().await.unwrap();
    assert_eq!(seen, heads[..4].iter().map(|e| e.id).collect::<Vec<_>>());

    // Predicates apply inside the snapshot.
    let mut filtered =
        f.db.open_artifact_snapshot(f.community, reader.as_bytes(), &[f.a])
            .await
            .unwrap()
            .unwrap();
    let none = filtered
        .next_page(&[("type".into(), vec!["other".into()])], 10)
        .await
        .unwrap();
    assert!(none.is_empty());

    // A private channel the reader cannot read refuses the whole sync.
    assert!(matches!(
        f.db.open_artifact_snapshot(f.community, f.peer.public_key().as_bytes(), &[f.a, f.b])
            .await
            .unwrap(),
        Err(super::artifact_query::SnapshotDenied::Restricted)
    ));
    // So does a channel that does not exist.
    assert!(matches!(
        f.db.open_artifact_snapshot(f.community, reader.as_bytes(), &[Uuid::new_v4()])
            .await
            .unwrap(),
        Err(super::artifact_query::SnapshotDenied::Restricted)
    ));

    let (head, is_deletion) =
        f.db.artifact_head(f.community, reader.as_bytes(), ids[4], &[])
            .await
            .unwrap()
            .unwrap();
    assert_eq!(head.event.id, deleted.id);
    assert!(is_deletion);
    let (head, is_deletion) =
        f.db.artifact_head(f.community, reader.as_bytes(), ids[0], &[])
            .await
            .unwrap()
            .unwrap();
    assert_eq!(head.event.id, heads[0].id);
    assert!(!is_deletion);
    let (view, _) = f
        .query(
            &f.owner,
            serde_json::json!({"artifact":"head","#d":[ids[4], ids[0]]}),
            false,
        )
        .await;
    let mut view: Vec<_> = view.iter().map(|e| e.event.id).collect();
    view.sort();
    let mut expected = vec![deleted.id, heads[0].id];
    expected.sort();
    assert_eq!(view, expected);
    assert_eq!(
        f.count(
            &f.owner,
            serde_json::json!({"artifact":"current","#d":[ids[4]]})
        )
        .await,
        0
    );
    // A reader who cannot read the head's channel learns nothing.
    let private = Uuid::new_v4();
    let create = f.revision(private, "create", f.b, None, &f.owner, vec![]);
    assert!(matches!(
        f.accept(&create, None).await,
        ArtifactOutcome::Accepted(_)
    ));
    assert!(f
        .db
        .artifact_head(f.community, f.peer.public_key().as_bytes(), private, &[])
        .await
        .unwrap()
        .is_none());
}

mod community {
    use super::*;
    use crate::community_artifact::{
        set_policy_in_transaction, CommunityArtifactPrincipal, PolicyOutcome,
    };
    use serde_json::json;

    async fn contract(f: &Fixture) {
        let mut connection = f.db.pool.acquire().await.unwrap();
        sqlx::query(
            "SET buzz.community_artifact_contract_ready='all-backends-enforce-community-audience'",
        )
        .execute(&mut *connection)
        .await
        .unwrap();
        sqlx::raw_sql(include_str!(
            "../../../../scripts/maintenance/community-artifact-contract.sql"
        ))
        .execute(&mut *connection)
        .await
        .unwrap();
        assert!(f.db.community_artifact_schema_ready().await.unwrap());
    }
    fn principal(keys: &Keys, roster: bool) -> CommunityArtifactPrincipal {
        CommunityArtifactPrincipal {
            owner_admission: true,
            pubkey: keys.public_key().to_bytes().to_vec(),
            verified_owner: None,
            require_membership: roster,
        }
    }
    fn revision(f: &Fixture, d: Uuid, op: &str, prev: Option<&Event>, keys: &Keys) -> Event {
        let mut tags = vec![
            vec!["ar".into(), "1".into()],
            vec!["d".into(), d.to_string()],
            vec!["community".into(), f.community.to_string()],
            vec!["type".into(), "example.settings".into()],
            vec!["op".into(), op.into()],
        ];
        if op != "delete" {
            tags.push(vec!["title".into(), "Settings".into()]);
        }
        if let Some(prev) = prev {
            tags.push(vec!["prev".into(), prev.id.to_hex()]);
        }
        EventBuilder::new(
            Kind::Custom(45010),
            if op == "delete" {
                ""
            } else {
                "community private needle"
            },
        )
        .tags(tags.into_iter().map(|t| Tag::parse(t).unwrap()))
        .sign_with_keys(keys)
        .unwrap()
    }
    fn policy(f: &Fixture, ty: &str, value: &str, prev: Option<&Event>) -> Event {
        EventBuilder::new(Kind::Custom(9034), "")
            .tags([
                Tag::parse(["community", &f.community.to_string()]).unwrap(),
                Tag::parse(["type", ty]).unwrap(),
                Tag::parse(["policy", value]).unwrap(),
                Tag::parse([
                    "prev",
                    &prev.map(|e| e.id.to_hex()).unwrap_or_else(|| "0".into()),
                ])
                .unwrap(),
            ])
            .sign_with_keys(&f.owner)
            .unwrap()
    }
    async fn set_policy(f: &Fixture, event: &Event) -> PolicyOutcome {
        let mut tx = f.db.pool.begin().await.unwrap();
        let out = set_policy_in_transaction(
            &mut tx,
            f.community,
            &principal(&f.owner, false),
            event,
            &artifact::parse_policy_command(event).unwrap(),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        out
    }
    async fn role(f: &Fixture, keys: &Keys, value: &str) {
        sqlx::query("INSERT INTO relay_members(community_id,pubkey,role) VALUES($1,$2,$3) ON CONFLICT(community_id,pubkey) DO UPDATE SET role=EXCLUDED.role")
            .bind(f.community.as_uuid()).bind(keys.public_key().to_hex()).bind(value).execute(&f.db.pool).await.unwrap();
    }
    async fn accept(
        f: &Fixture,
        event: &Event,
        reader: &CommunityArtifactPrincipal,
    ) -> crate::Result<ArtifactOutcome> {
        f.db.accept_community_artifact(
            f.community,
            reader,
            event,
            &artifact::validate(event).unwrap(),
        )
        .await
    }
    async fn read(
        f: &Fixture,
        reader: &CommunityArtifactPrincipal,
        view: &str,
        d: Uuid,
    ) -> crate::Result<Vec<buzz_core::StoredEvent>> {
        f.db.query_community_artifacts(
            f.community,
            reader,
            &artifact::parse_query(&json!({"artifact":view,"scope":"community","#d":[d]})).unwrap(),
        )
        .await
    }

    /// Mutation: dropping the old primary key in expand makes the legacy
    /// ON CONFLICT below fail. Mutation: omitting scope from a head/history
    /// predicate aliases the legal identical d after the operator contract.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn migration_schema_expand_keeps_legacy_conflict_target_contract_separates_scopes() {
        let f = Fixture::new().await;
        let d = Uuid::new_v4();
        let channel = f.revision(d, "create", f.a, None, &f.owner, vec![]);
        assert!(matches!(
            f.accept(&channel, None).await,
            ArtifactOutcome::Accepted(_)
        ));
        // Exercise the documented pre-expansion/concurrent-prebuild procedure
        // on populated tables, including recovery from rolled-back columns.
        sqlx::raw_sql("ALTER TABLE artifact_heads DROP COLUMN scope; ALTER TABLE artifact_heads ALTER COLUMN channel_id SET NOT NULL; ALTER TABLE artifact_revisions DROP COLUMN scope, DROP COLUMN community_event, DROP COLUMN redacted; ALTER TABLE communities DROP COLUMN artifact_type_policies;")
            .execute(&f.db.pool).await.unwrap();
        let expand = include_str!("../../../../migrations/0097_community_artifact_expand.sql");
        let additive = expand.split("-- Small deployments").next().unwrap();
        let mut tx = f.db.pool.begin().await.unwrap();
        sqlx::raw_sql(additive).execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        for sql in [
            "CREATE UNIQUE INDEX CONCURRENTLY artifact_heads_scoped_identity ON artifact_heads(community_id,scope,artifact_id)",
            "CREATE INDEX CONCURRENTLY artifact_revisions_scoped_identity ON artifact_revisions(community_id,scope,artifact_id)",
            "CREATE INDEX CONCURRENTLY artifact_heads_live_type ON artifact_heads(community_id,scope,artifact_type,artifact_id) WHERE NOT deleted",
        ] { sqlx::query(sql).execute(&f.db.pool).await.unwrap(); }
        let mut tx = f.db.pool.begin().await.unwrap();
        sqlx::raw_sql(expand).execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        // The exact legacy upsert target and output columns still work on the
        // expanded catalog, even on a populated table.
        let output:(Vec<u8>,Uuid)=sqlx::query_as("INSERT INTO artifact_heads(community_id,artifact_id,event_id,channel_id,artifact_type) VALUES($1,$2,$3,$4,'buzz.task') ON CONFLICT(community_id,artifact_id) DO UPDATE SET event_id=EXCLUDED.event_id RETURNING event_id,channel_id")
            .bind(f.community.as_uuid()).bind(d).bind(channel.id.as_bytes().as_slice()).bind(f.a).fetch_one(&f.db.pool).await.unwrap();
        assert_eq!(output, (channel.id.as_bytes().to_vec(), f.a));
        assert!(!f.db.community_artifact_schema_ready().await.unwrap());
        let mut conn = f.db.pool.acquire().await.unwrap();
        assert!(sqlx::raw_sql(include_str!(
            "../../../../scripts/maintenance/community-artifact-contract.sql"
        ))
        .execute(&mut *conn)
        .await
        .is_err());
        sqlx::query("ROLLBACK").execute(&mut *conn).await.unwrap();
        drop(conn);
        contract(&f).await;
        let community = revision(&f, d, "create", None, &f.owner);
        let reader = principal(&f.owner, false);
        assert!(matches!(
            accept(&f, &community, &reader).await.unwrap(),
            ArtifactOutcome::Accepted(_)
        ));
        contract(&f).await;
        let channel_update = f.revision(d, "update", f.a, Some(&channel), &f.owner, vec![]);
        assert!(matches!(
            f.accept(&channel_update, None).await,
            ArtifactOutcome::Accepted(_)
        ));
        assert_eq!(
            read(&f, &reader, "head", d).await.unwrap()[0].event.id,
            community.id
        );
        assert_eq!(read(&f, &reader, "history", d).await.unwrap().len(), 1);
        let (legacy, _) = f
            .query(&f.owner, json!({"artifact":"history","#d":[d]}), false)
            .await;
        assert_eq!(legacy.len(), 2);
        assert!(legacy.iter().all(|e| e.channel_id == Some(f.a)));
        let deleted = revision(&f, d, "delete", Some(&community), &f.owner);
        assert!(matches!(
            accept(&f, &deleted, &reader).await.unwrap(),
            ArtifactOutcome::Accepted(_)
        ));
        assert!(read(&f, &reader, "current", d).await.unwrap().is_empty());
        assert_eq!(
            read(&f, &reader, "head", d).await.unwrap()[0].event.id,
            deleted.id
        );
        let restore = revision(&f, d, "restore", Some(&deleted), &f.owner);
        assert!(matches!(
            accept(&f, &restore, &reader).await.unwrap(),
            ArtifactOutcome::Accepted(_)
        ));
        role(&f, &f.owner, "owner").await;
        assert!(f
            .db
            .redact_community_artifact(f.community, &reader, restore.id.as_bytes())
            .await
            .unwrap()
            .is_some());
        assert!(read(&f, &reader, "head", d).await.unwrap().is_empty());
        assert!(f
            .db
            .community_artifact_accepted(f.community, &reader, &restore)
            .await
            .unwrap());
        assert_eq!(
            f.count(&f.owner, json!({"artifact":"current","#d":[d]}))
                .await,
            1
        );
        f.db.validate_deletion_serving_catalog().await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn policy_cas_race_replay_cap_and_retained_everyone() {
        let f = Fixture::new().await;
        contract(&f).await;
        role(&f, &f.owner, "owner").await;
        let reader = principal(&f.owner, false);
        let unset =
            f.db.community_artifact_policy(f.community, &reader, "example.settings")
                .await
                .unwrap();
        assert_eq!(unset.policy, "everyone");
        assert_eq!(unset.revision, None);
        let a = policy(&f, "example.settings", "admins", None);
        let b = policy(&f, "example.settings", "everyone", None);
        let (ra, rb) = tokio::join!(set_policy(&f, &a), set_policy(&f, &b));
        let winner = match (ra, rb) {
            (PolicyOutcome::Applied, PolicyOutcome::Conflict(_)) => &a,
            (PolicyOutcome::Conflict(_), PolicyOutcome::Applied) => &b,
            x => panic!("one winner: {x:?}"),
        };
        assert_eq!(set_policy(&f, winner).await, PolicyOutcome::Duplicate);
        let everyone = policy(&f, "example.settings", "everyone", Some(winner));
        assert_eq!(set_policy(&f, &everyone).await, PolicyOutcome::Applied);
        assert!(matches!(
            set_policy(&f, winner).await,
            PolicyOutcome::Conflict(_)
        ));
        let state =
            f.db.community_artifact_policy(f.community, &reader, "example.settings")
                .await
                .unwrap();
        assert_eq!(state.policy, "everyone");
        assert_eq!(state.revision, Some(everyone.id.to_hex()));
        // Fill the bounded map efficiently, then exercise the production CAS
        // for both an existing slot and a new explicit everyone slot.
        let entries: serde_json::Map<String, serde_json::Value> = (0..255)
            .map(|i| {
                (
                    format!("example.slot{i}"),
                    json!({"policy":"admins","revision":"a".repeat(64)}),
                )
            })
            .chain(std::iter::once((
                "example.settings".into(),
                json!({"policy":"everyone","revision":everyone.id.to_hex()}),
            )))
            .collect();
        sqlx::query("UPDATE communities SET artifact_type_policies=$2 WHERE id=$1")
            .bind(f.community.as_uuid())
            .bind(serde_json::Value::Object(entries))
            .execute(&f.db.pool)
            .await
            .unwrap();
        assert!(matches!(
            set_policy(&f, &policy(&f, "example.new", "everyone", None)).await,
            PolicyOutcome::Conflict(_)
        ));
        let update = policy(&f, "example.settings", "admins", Some(&everyone));
        assert_eq!(set_policy(&f, &update).await, PolicyOutcome::Applied);
        let event = revision(&f, Uuid::new_v4(), "create", None, &f.peer);
        assert!(accept(&f, &event, &principal(&f.peer, false))
            .await
            .is_err());
        let mut delegated = principal(&f.peer, true);
        delegated.verified_owner = Some(reader.pubkey.clone());
        // Delegation gives admission, never the admin edit role.
        assert!(accept(&f, &event, &delegated).await.is_err());
        assert!(f
            .db
            .community_artifact_policy(f.community, &delegated, "example.settings")
            .await
            .is_ok());
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn policy_transaction_rechecks_timeout_before_writes_and_replays() {
        let f = Fixture::new().await;
        contract(&f).await;
        role(&f, &f.owner, "owner").await;
        let current = policy(&f, "example.settings", "admins", None);
        assert_eq!(set_policy(&f, &current).await, PolicyOutcome::Applied);
        sqlx::query("INSERT INTO community_bans(community_id,pubkey,actor_pubkey,muted_until) VALUES($1,$2,$2,now()+interval '1 hour')")
            .bind(f.community.as_uuid())
            .bind(f.owner.public_key().as_bytes().as_slice())
            .execute(&f.db.pool)
            .await
            .unwrap();
        let next = policy(&f, "example.settings", "everyone", Some(&current));
        for event in [&current, &next] {
            let command = buzz_core::artifact::parse_policy_command(event).unwrap();
            let mut tx = f.db.pool.begin().await.unwrap();
            let result = set_policy_in_transaction(
                &mut tx,
                f.community,
                &principal(&f.owner, false),
                event,
                &command,
            )
            .await;
            assert!(
                matches!(result, Err(crate::DbError::AccessDenied(_))),
                "{result:?}"
            );
            tx.rollback().await.unwrap();
        }
    }

    /// Admission must be authoritative even with a caller holding stale
    /// transport ownership/membership. Accepted replay ignores timeout and
    /// policy, but never tenant or ban state.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn replay_and_reads_recheck_admission_and_applicable_owner() {
        let f = Fixture::new().await;
        contract(&f).await;
        role(&f, &f.owner, "owner").await;
        let reader = principal(&f.peer, false);
        let d = Uuid::new_v4();
        let event = revision(&f, d, "create", None, &f.peer);
        assert!(matches!(
            accept(&f, &event, &reader).await.unwrap(),
            ArtifactOutcome::Accepted(_)
        ));
        let command = policy(&f, "example.settings", "admins", None);
        assert_eq!(set_policy(&f, &command).await, PolicyOutcome::Applied);
        sqlx::query("INSERT INTO community_bans(community_id,pubkey,actor_pubkey,muted_until) VALUES($1,$2,$3,now()+interval '1 hour')")
            .bind(f.community.as_uuid()).bind(&reader.pubkey).bind(f.owner.public_key().as_bytes().as_slice()).execute(&f.db.pool).await.unwrap();
        assert!(f
            .db
            .community_artifact_accepted(f.community, &reader, &event)
            .await
            .unwrap());
        assert_eq!(read(&f, &reader, "current", d).await.unwrap().len(), 1);
        let mut roster = reader.clone();
        roster.require_membership = true;
        assert!(read(&f, &roster, "current", d).await.is_err());
        role(&f, &f.peer, "member").await;
        assert_eq!(read(&f, &roster, "current", d).await.unwrap().len(), 1);
        sqlx::query("DELETE FROM relay_members WHERE community_id=$1 AND pubkey=$2")
            .bind(f.community.as_uuid())
            .bind(f.peer.public_key().to_hex())
            .execute(&f.db.pool)
            .await
            .unwrap();
        assert!(read(&f, &roster, "current", d).await.is_err());
        roster.verified_owner = Some(f.owner.public_key().to_bytes().to_vec());
        assert_eq!(read(&f, &roster, "current", d).await.unwrap().len(), 1);
        sqlx::query("INSERT INTO community_bans(community_id,pubkey,actor_pubkey,banned) VALUES($1,$2,$2,true)")
            .bind(f.community.as_uuid()).bind(f.owner.public_key().as_bytes().as_slice()).execute(&f.db.pool).await.unwrap();
        assert!(read(&f, &roster, "current", d).await.is_err());
        sqlx::query("UPDATE community_bans SET banned=true WHERE community_id=$1")
            .bind(f.community.as_uuid())
            .execute(&f.db.pool)
            .await
            .unwrap();
        assert!(f
            .db
            .community_artifact_accepted(f.community, &reader, &event)
            .await
            .is_err());
        let wrong = CommunityId::from_uuid(Uuid::new_v4());
        assert!(f
            .db
            .community_artifact_accepted(wrong, &reader, &event)
            .await
            .is_err());
    }

    /// The enqueue guard shares the restriction-writer lock and member row
    /// locks: a revocation commits on one side of the guarded enqueue, never
    /// between its admission check and queue point.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn read_guard_serializes_committed_revocation() {
        let f = Fixture::new().await;
        // One snapshot in a two-slot pool must leave room for fresh admission.
        // A second snapshot sheds instead of taking the last writer slot.
        let small = Db::from_pool(
            sqlx::postgres::PgPoolOptions::new()
                .max_connections(2)
                .connect(&crate::test_support::database_url())
                .await
                .unwrap(),
        );
        let open = principal(&f.peer, false);
        let snapshot = small
            .open_community_artifact_snapshot(f.community, &open)
            .await
            .unwrap();
        assert!(tokio::time::timeout(
            std::time::Duration::from_secs(2),
            small.open_community_artifact_snapshot(f.community, &open)
        )
        .await
        .unwrap()
        .is_err());
        small
            .community_artifact_read_guard(f.community, &open)
            .await
            .unwrap()
            .finish()
            .await
            .unwrap();
        snapshot.finish().await.unwrap();
        let one = Db::from_pool(
            sqlx::postgres::PgPoolOptions::new()
                .max_connections(1)
                .connect(&crate::test_support::database_url())
                .await
                .unwrap(),
        );
        assert!(one
            .open_community_artifact_snapshot(f.community, &open)
            .await
            .is_err());
        role(&f, &f.peer, "member").await;
        let reader = principal(&f.peer, true);
        let guard =
            f.db.community_artifact_read_guard(f.community, &reader)
                .await
                .unwrap();
        let pool = f.db.pool.clone();
        let community = f.community;
        let key = f.peer.public_key().to_hex();
        let removal = tokio::spawn(async move {
            sqlx::query("DELETE FROM relay_members WHERE community_id=$1 AND pubkey=$2")
                .bind(community.as_uuid())
                .bind(key)
                .execute(&pool)
                .await
                .unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!removal.is_finished());
        guard.finish().await.unwrap();
        removal.await.unwrap();
        assert!(f
            .db
            .community_artifact_read_guard(f.community, &reader)
            .await
            .is_err());
        role(&f, &f.peer, "member").await;
        role(&f, &f.owner, "owner").await;
        let guard =
            f.db.community_artifact_read_guard(f.community, &reader)
                .await
                .unwrap();
        let pool = f.db.pool.clone();
        let archive = tokio::spawn(async move {
            sqlx::query("UPDATE communities SET archived_at=clock_timestamp() WHERE id=$1")
                .bind(community.as_uuid())
                .execute(&pool)
                .await
                .unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            !archive.is_finished(),
            "archive cannot commit before guarded enqueue"
        );
        guard.finish().await.unwrap();
        archive.await.unwrap();
        let open_reader = principal(&f.peer, false);
        assert!(f
            .db
            .community_artifact_read_guard(f.community, &open_reader)
            .await
            .is_err());
        let event = revision(&f, Uuid::new_v4(), "create", None, &f.peer);
        assert!(accept(&f, &event, &open_reader).await.is_err());
        let command = policy(&f, "example.settings", "admins", None);
        let parsed = buzz_core::artifact::parse_policy_command(&command).unwrap();
        let mut tx = f.db.pool.begin().await.unwrap();
        assert!(set_policy_in_transaction(
            &mut tx,
            f.community,
            &principal(&f.owner, false),
            &command,
            &parsed
        )
        .await
        .is_err());
    }
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn cluster_global_artifact_admission_and_owner_transfer_both_finish() {
        use crate::relay_members::{transfer_ownership, TransferResult};
        use crate::test_support::{named_pool, wait_for_lock_wait};

        let f = Fixture::new().await;
        role(&f, &f.owner, "owner").await;
        role(&f, &f.peer, "member").await;

        // Pause the transfer after it takes the community row and before it
        // promotes the transferee. Admission for that member must wait without
        // holding a member lock the transfer needs.
        let mut blocker = f.db.pool.begin().await.unwrap();
        let owner = f.owner.public_key().to_hex();
        sqlx::query("SELECT 1 FROM relay_members WHERE community_id=$1 AND pubkey=$2 FOR UPDATE")
            .bind(f.community.as_uuid())
            .bind(&owner)
            .fetch_one(&mut *blocker)
            .await
            .unwrap();

        let transfer_name = format!("artifact-transfer-{}", Uuid::new_v4().simple());
        let transfer_pool = named_pool(&transfer_name).await;
        let read_name = format!("artifact-admission-{}", Uuid::new_v4().simple());
        let reader_db = Db::from_pool(named_pool(&read_name).await);
        let community = f.community;
        let peer = f.peer.public_key().to_hex();
        let relay = f.relay.clone();
        let transfer = tokio::spawn(async move {
            transfer_ownership(&transfer_pool, community, &peer, &owner, &relay).await
        });
        wait_for_lock_wait(&f.db.pool, &transfer_name).await;

        let reader = principal(&f.peer, true);
        let admission = tokio::spawn(async move {
            reader_db
                .community_artifact_read_guard(community, &reader)
                .await?
                .finish()
                .await
        });
        wait_for_lock_wait(&f.db.pool, &read_name).await;
        blocker.rollback().await.unwrap();

        let result = transfer
            .await
            .unwrap()
            .expect("ownership transfer succeeds");
        assert!(matches!(result.outcome, TransferResult::Transferred { .. }));
        admission
            .await
            .unwrap()
            .expect("artifact admission succeeds");
    }

    /// A policy update's uncommitted state must fence new artifact admission,
    /// even when that type had no previous policy row.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn unset_policy_lock_serializes_artifact_admission() {
        let f = Fixture::new().await;
        contract(&f).await;
        role(&f, &f.owner, "owner").await;
        let command = policy(&f, "example.settings", "admins", None);
        let parsed = buzz_core::artifact::parse_policy_command(&command).unwrap();
        let mut tx = f.db.pool.begin().await.unwrap();
        assert_eq!(
            set_policy_in_transaction(
                &mut tx,
                f.community,
                &principal(&f.owner, false),
                &command,
                &parsed
            )
            .await
            .unwrap(),
            PolicyOutcome::Applied
        );
        let event = revision(&f, Uuid::new_v4(), "create", None, &f.peer);
        let env = buzz_core::artifact::validate(&event).unwrap();
        let db = f.db.clone();
        let community = f.community;
        let peer = principal(&f.peer, false);
        let pending = tokio::spawn(async move {
            db.accept_community_artifact(community, &peer, &event, &env)
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            !pending.is_finished(),
            "artifact bypassed the type policy transaction"
        );
        tx.commit().await.unwrap();
        assert!(
            pending.await.unwrap().is_err(),
            "committed admins policy must reject member write"
        );
    }
}
