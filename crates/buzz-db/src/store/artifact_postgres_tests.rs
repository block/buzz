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

/// Exercise the production Move transaction, including its second, relay-signed
/// insert after domain row writes. Exclusive holders do not touch domain rows.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn artifact_move_and_identity_fences_finish_without_partial_writes_or_deadlocks() {
    use crate::user_deletion_fences::{self as fences, FenceIdentity};
    use std::{sync::Arc, time::Duration};
    for block_relay in [false, true] {
        let f = Arc::new(Fixture::new().await);
        let d = Uuid::new_v4();
        let create = f.revision(d, "create", f.a, None, &f.owner, vec![]);
        f.accept(&create, None).await;
        let moved = f.revision(d, "move", f.b, Some(&create), &f.owner, vec![]);
        let move_id = moved.id;
        let store = f.db.entity_deletion_store();
        let request =
            f.db.user_deletion_store()
                .submit(&format!("demo:{}", Uuid::new_v4()), "operator", None)
                .await
                .unwrap();
        store
            .approve(
                request.id,
                "reviewer",
                request.inventory_digest.as_deref().unwrap(),
            )
            .await
            .unwrap();
        let lease = store
            .claim(request.id, "worker", Duration::from_secs(300))
            .await
            .unwrap()
            .unwrap();
        let identity = FenceIdentity::user(f.owner.public_key());
        let mut gate = f.db.pool.begin().await.unwrap();
        if block_relay {
            fences::lock_exclusive(&mut gate, &[FenceIdentity::user(f.relay.public_key())])
                .await
                .unwrap();
        } else {
            fences::insert(&mut gate, identity, &lease, None)
                .await
                .unwrap();
        }
        let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *gate)
            .await
            .unwrap();
        let task_f = f.clone();
        let task = tokio::spawn(async move {
            let env = artifact::validate(&moved).unwrap();
            task_f
                .db
                .accept_artifact(
                    task_f.community,
                    &moved,
                    &env,
                    Some(task_f.a),
                    &task_f.relay,
                )
                .await
        });
        wait_for_artifact_blocker(&f.db.pool, gate_pid).await;
        let writer_pid: i32 = sqlx::query_scalar("SELECT pid FROM pg_stat_activity WHERE datname=current_database() AND $1=ANY(pg_blocking_pids(pid)) LIMIT 1")
            .bind(gate_pid).fetch_one(&f.db.pool).await.unwrap();
        let coordinate_held: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_locks WHERE pid=$1 AND locktype='advisory' AND granted AND mode='ExclusiveLock'
             AND classid=((hashtextextended($2,0) >> 32) & 4294967295)::oid AND objid=(hashtextextended($2,0) & 4294967295)::oid)")
            .bind(writer_pid).bind(format!("artifact:{}:{}", f.community.as_uuid(), d)).fetch_one(&f.db.pool).await.unwrap();
        assert!(
            coordinate_held,
            "actual artifact caller must hold its domain lock before waiting"
        );
        let pending_user_fence = if block_relay {
            let mut user_gate = f.db.pool.begin().await.unwrap();
            let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *user_gate)
                .await
                .unwrap();
            let fence_task = tokio::spawn(async move {
                fences::insert(&mut user_gate, identity, &lease, None)
                    .await
                    .unwrap();
                user_gate.commit().await.unwrap();
            });
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    let blockers: Vec<i32> = sqlx::query_scalar("SELECT pg_blocking_pids($1)")
                        .bind(pid)
                        .fetch_one(&f.db.pool)
                        .await
                        .unwrap();
                    if blockers.contains(&writer_pid) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("fence must wait on Move's already acquired author lock");
            Some(fence_task)
        } else {
            None
        };
        gate.commit().await.unwrap();
        let outcome = tokio::time::timeout(Duration::from_secs(8), task)
            .await
            .unwrap()
            .unwrap();
        if let Some(task) = pending_user_fence {
            tokio::time::timeout(Duration::from_secs(8), task)
                .await
                .unwrap()
                .unwrap();
        }
        if block_relay {
            let ArtifactOutcome::Accepted(events) = outcome.unwrap() else {
                panic!("Move should finish before queued author fence");
            };
            assert_eq!(events.len(), 2);
            assert_eq!(events[1].event.kind.as_u16(), 45011);
        } else {
            assert!(matches!(
                outcome,
                Err(crate::DbError::UserDeletionRestricted)
            ));
        }
        let head: Vec<u8> = sqlx::query_scalar(
            "SELECT event_id FROM artifact_heads WHERE community_id=$1 AND artifact_id=$2",
        )
        .bind(f.community.as_uuid())
        .bind(d)
        .fetch_one(&f.db.pool)
        .await
        .unwrap();
        assert_eq!(
            head,
            if block_relay {
                move_id.as_bytes()
            } else {
                create.id.as_bytes()
            }
        );
        let revision_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM artifact_revisions WHERE community_id=$1 AND event_id=$2)",
        )
        .bind(f.community.as_uuid())
        .bind(move_id.as_bytes().as_slice())
        .fetch_one(&f.db.pool)
        .await
        .unwrap();
        assert_eq!(revision_exists, block_relay);
        let removals: i64 =
            sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND kind=45011")
                .bind(f.community.as_uuid())
                .fetch_one(&f.db.pool)
                .await
                .unwrap();
        assert_eq!(removals, i64::from(block_relay));
        let deadlocks: i64 = sqlx::query_scalar(
            "SELECT deadlocks FROM pg_stat_database WHERE datname=current_database()",
        )
        .fetch_one(&f.db.pool)
        .await
        .unwrap();
        assert_eq!(deadlocks, 0);
    }
}

async fn wait_for_artifact_blocker(pool: &PgPool, pid: i32) {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND $1=ANY(pg_blocking_pids(pid)))")
                .bind(pid).fetch_one(pool).await.unwrap();
            if waiting { return; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.expect("artifact writer must wait on identity lock");
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
