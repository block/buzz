//! Functional coverage through admitted production deletion entry points.

use super::*;
use crate::{event::ThreadMetadataParams, Db};
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
use sqlx::postgres::PgPoolOptions;

const JAN: u64 = 1_768_003_200; // 2026-01-10 UTC
const MAY: u64 = 1_778_371_200; // 2026-05-10 UTC

async fn fixture() -> (Db, CommunityId, Uuid) {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&crate::test_support::database_url())
        .await
        .expect("test database");
    let db = Db::from_pool(pool);
    let (community, channel) = tenant(&db.pool).await;
    (db, community, channel)
}

async fn tenant(pool: &PgPool) -> (CommunityId, Uuid) {
    let community = Uuid::new_v4();
    let channel = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(community)
        .bind(format!("deletion-{}.example", community.simple()))
        .execute(pool)
        .await
        .expect("community");
    sqlx::query(
        "INSERT INTO channels (id, community_id, name, created_by) VALUES ($1,$2,'deletion',$3)",
    )
    .bind(channel)
    .bind(community)
    .bind(vec![7_u8; 32])
    .execute(pool)
    .await
    .expect("channel");
    (CommunityId::from_uuid(community), channel)
}

fn event(kind: u16, secs: u64, tags: Vec<Tag>) -> nostr::Event {
    EventBuilder::new(Kind::Custom(kind), "deletion test")
        .tags(tags)
        .custom_created_at(Timestamp::from(secs))
        .sign_with_keys(&Keys::generate())
        .expect("signed event")
}

/// Seed `event` as a row already in storage. The serving writers reject a
/// channel event older than the `created_at` floor, and these tests pin
/// months far in the past; the deletion paths under test still run for real.
async fn seed(db: &Db, community: CommunityId, event: &nostr::Event, channel: Uuid) {
    crate::test_support::insert_historical_event(&db.pool, community, event, channel).await;
}

/// [`seed`] for a reply or root that also carries thread metadata.
async fn seed_with_thread(
    db: &Db,
    community: CommunityId,
    event: &nostr::Event,
    channel: Uuid,
    meta: ThreadMetadataParams<'_>,
) {
    crate::test_support::insert_historical_event_with_thread_metadata(
        &db.pool, community, event, channel, meta,
    )
    .await;
}

fn timestamp(event: &nostr::Event) -> DateTime<Utc> {
    DateTime::from_timestamp(event.created_at.as_secs() as i64, 0).expect("timestamp")
}

fn target(event: &nostr::Event) -> DeletionTarget<'_> {
    DeletionTarget {
        event_id: event.id.as_bytes(),
        created_at: timestamp(event),
        parent_event_id: None,
        root_event_id: None,
    }
}

async fn counts(db: &Db, community: CommunityId, id: &[u8]) -> (i64, i64, i64) {
    let (rows, live): (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(*) FILTER (WHERE deleted_at IS NULL) FROM events WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid()).bind(id).fetch_one(&db.pool).await.expect("event counts");
    let mentions = sqlx::query_scalar(
        "SELECT count(*) FROM event_mentions WHERE community_id=$1 AND event_id=$2",
    )
    .bind(community.as_uuid())
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .expect("mention counts");
    (rows, live, mentions)
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn known_targets_across_months_are_exact_and_tenant_scoped() {
    let (db, community, channel) = fixture().await;
    let (other, other_channel) = tenant(&db.pool).await;
    let jan = event(7, JAN, vec![]);
    let may = event(7, MAY, vec![]);
    for ev in [&jan, &may] {
        seed(&db, community, ev, channel).await;
        seed(&db, other, ev, other_channel).await;
    }
    // Deliberately older than both targets: e-target deletion has no inferred
    // chronology, and the deletion's month is not a target key. It is stored
    // without a channel, since a channel event this old is below the floor.
    let deletion = event(5, JAN - 86_400, vec![]);
    let missing = event(7, JAN + 1, vec![]);
    let wrong = DeletionTarget {
        created_at: timestamp(&may),
        ..target(&jan)
    };
    let (_, inserted, deleted) = db
        .insert_deletion_event(
            community,
            &deletion,
            None,
            None,
            &[wrong, target(&missing), target(&jan), target(&may)],
        )
        .await
        .expect("delete targets");
    assert!(inserted);
    assert_eq!(deleted, [false, false, true, true]);
    for ev in [&jan, &may] {
        assert_eq!(counts(&db, community, ev.id.as_bytes()).await.1, 0);
        assert_eq!(counts(&db, other, ev.id.as_bytes()).await.1, 1);
    }
    assert_eq!(
        db.insert_deletion_event(
            community,
            &deletion,
            None,
            None,
            &[target(&jan), target(&may)]
        )
        .await
        .expect("replay")
        .2,
        [false, false]
    );
    // ID-only callers still work and discover the immutable timestamp in tx.
    assert!(
        soft_delete_event_and_update_thread(&db.pool, other, jan.id.as_bytes(), None, None)
            .await
            .expect("compatibility delete")
    );
    assert!(!soft_delete_event_and_update_thread(
        &db.pool,
        other,
        missing.id.as_bytes(),
        None,
        None
    )
    .await
    .expect("missing compatibility target"));
}

async fn retention_free_contract() {
    let (db, community, channel) = fixture().await;
    let (other, other_channel) = tenant(&db.pool).await;
    let slot = format!("read-state:{}", "b".repeat(32));
    let marker = event(45011, JAN, vec![]);
    seed(&db, community, &marker, channel).await;
    for (kind, d, extra) in [
        (30078, slot.as_str(), ["t", "read-state"]),
        (
            30003,
            "buzz-mesh-member-status:test",
            ["k", "buzz-mesh-status"],
        ),
    ] {
        for secs in [JAN, MAY] {
            let tags = vec![
                Tag::parse(["d", d]).expect("d"),
                Tag::parse(extra).expect("classification"),
                Tag::parse(["p", &"c".repeat(64)]).expect("mention"),
            ];
            let ev = event(kind, secs, tags);
            for (tenant, ch) in [(community, channel), (other, other_channel)] {
                // Each event has its own author, so nothing is superseded and
                // storing it is the whole replacement.
                seed(&db, tenant, &ev, ch).await;
                assert_eq!(counts(&db, tenant, ev.id.as_bytes()).await, (1, 1, 1));
            }
            let deletion = event(5, JAN - 1, vec![]);
            let wrong = DeletionTarget {
                created_at: timestamp(&ev) + chrono::Duration::seconds(1),
                ..target(&ev)
            };
            assert_eq!(
                db.insert_deletion_event(community, &deletion, None, None, &[wrong])
                    .await
                    .expect("wrong timestamp")
                    .2,
                [false]
            );
            assert_eq!(counts(&db, community, ev.id.as_bytes()).await, (1, 1, 1));
            // A later forbidden target must roll back both the physical purge
            // and its mention cleanup, including the migration NIP-RS guard.
            assert!(db
                .insert_deletion_event(
                    community,
                    &deletion,
                    None,
                    None,
                    &[target(&ev), target(&marker)]
                )
                .await
                .is_err());
            assert_eq!(counts(&db, community, ev.id.as_bytes()).await, (1, 1, 1));
            assert_eq!(
                db.insert_deletion_event(community, &deletion, None, None, &[target(&ev)])
                    .await
                    .expect("hard purge")
                    .2,
                [true]
            );
            assert_eq!(counts(&db, community, ev.id.as_bytes()).await, (0, 0, 0));
            assert_eq!(counts(&db, other, ev.id.as_bytes()).await, (1, 1, 1));
            assert_eq!(
                db.insert_deletion_event(community, &deletion, None, None, &[target(&ev)])
                    .await
                    .expect("replay purge")
                    .2,
                [false]
            );
            assert!(soft_delete_event_and_update_thread(
                &db.pool,
                other,
                ev.id.as_bytes(),
                None,
                None
            )
            .await
            .expect("ID-only purge"));
            assert_eq!(counts(&db, other, ev.id.as_bytes()).await, (0, 0, 0));
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn known_target_retention_free_purge_cleans_mentions() {
    retention_free_contract().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_known_target_retention_free_purge_cleans_mentions() {
    crate::migration::run_migrations(
        &PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("migration pool"),
    )
    .await
    .expect("migrate");
    retention_free_contract().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn known_target_batch_rollback_then_replay_repairs_and_counts_once() {
    let (db, community, channel) = fixture().await;
    let root = event(9, JAN, vec![]);
    seed_with_thread(
        &db,
        community,
        &root,
        channel,
        ThreadMetadataParams {
            event_id: root.id.as_bytes(),
            event_created_at: timestamp(&root),
            channel_id: channel,
            parent_event_id: None,
            parent_event_created_at: None,
            root_event_id: None,
            root_event_created_at: None,
            depth: 0,
            broadcast: false,
        },
    )
    .await;
    let reply = event(9, MAY, vec![]);
    seed_with_thread(
        &db,
        community,
        &reply,
        channel,
        ThreadMetadataParams {
            event_id: reply.id.as_bytes(),
            event_created_at: timestamp(&reply),
            channel_id: channel,
            parent_event_id: Some(root.id.as_bytes()),
            parent_event_created_at: Some(timestamp(&root)),
            root_event_id: Some(root.id.as_bytes()),
            root_event_created_at: Some(timestamp(&root)),
            depth: 1,
            broadcast: false,
        },
    )
    .await;
    let counter = || async {
        sqlx::query_as::<_, (i32, i32)>("SELECT reply_count, descendant_count FROM thread_metadata WHERE community_id=$1 AND event_id=$2")
            .bind(community.as_uuid()).bind(root.id.as_bytes().as_slice())
            .fetch_one(&db.pool).await.expect("counters")
    };
    assert_eq!(counter().await, (1, 1));
    // A forbidden removal marker makes the second target fail after the first
    // was mutated. No test-only triggers or copied deletion implementation.
    let marker = event(45011, JAN, vec![]);
    seed(&db, community, &marker, channel).await;
    let reply_target = DeletionTarget {
        parent_event_id: Some(root.id.as_bytes()),
        root_event_id: Some(root.id.as_bytes()),
        ..target(&reply)
    };
    // Stored without a channel on this attempt: a channel event this old is
    // below the floor, which would fail the write before the targets run.
    let deletion = event(5, MAY + 1, vec![]);
    assert!(db
        .insert_deletion_event(
            community,
            &deletion,
            None,
            None,
            &[reply_target, target(&marker)]
        )
        .await
        .is_err());
    assert_eq!(counts(&db, community, deletion.id.as_bytes()).await.0, 0);
    assert_eq!(counts(&db, community, reply.id.as_bytes()).await.1, 1);
    assert_eq!(counter().await, (1, 1));
    // Emulate a historical torn write, then concurrently replay it from two
    // independent connections: exactly one transition decrements counters.
    seed(&db, community, &deletion, channel).await;
    let concurrent = Db::from_pool(
        PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("independent pool"),
    );
    let targets = [reply_target];
    let (a, b) = tokio::join!(
        db.insert_deletion_event(community, &deletion, Some(channel), None, &targets),
        concurrent.insert_deletion_event(community, &deletion, Some(channel), None, &targets),
    );
    let (_, inserted_a, a) = a.expect("first replay");
    let (_, inserted_b, b) = b.expect("second replay");
    assert!(!inserted_a && !inserted_b);
    assert_ne!(a, b);
    assert_eq!(counter().await, (0, 0));
    assert_eq!(
        db.insert_deletion_event(community, &deletion, Some(channel), None, &targets)
            .await
            .expect("repeat replay")
            .2,
        [false]
    );
    assert_eq!(counter().await, (0, 0));
}

fn event_relations(plan: &serde_json::Value, relations: &mut Vec<String>) {
    if let Some(name) = plan.get("Relation Name").and_then(|name| name.as_str()) {
        // Generic plans can list nonexecuted target tables / subplans. Only
        // executed relation-access nodes measure runtime pruning.
        if name.starts_with("events_p")
            && plan
                .get("Actual Loops")
                .and_then(|loops| loops.as_u64())
                .unwrap_or(0)
                > 0
        {
            relations.push(name.to_owned());
        }
    }
    match plan {
        serde_json::Value::Array(items) => {
            for item in items {
                event_relations(item, relations);
            }
        }
        serde_json::Value::Object(items) => {
            for item in items.values() {
                event_relations(item, relations);
            }
        }
        _ => {}
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn production_known_target_statements_prune_to_one_month() {
    let (db, community, channel) = fixture().await;
    let slot = format!("read-state:{}", "a".repeat(32));
    let ev = event(
        30078,
        JAN,
        vec![
            Tag::parse(["d", &slot]).expect("d"),
            Tag::parse(["t", "read-state"]).expect("t"),
        ],
    );
    seed(&db, community, &ev, channel).await;
    let deletion = event(5, MAY, vec![]);
    db.insert_deletion_event(community, &deletion, None, None, &[target(&ev)])
        .await
        .expect("exercise production statements");
    // Explain the exact statements production executes, with their real
    // bindings. Production runs them unnamed (`.persistent(false)`), so they
    // are planned with the bound parameters; the lock test below covers the
    // generic-plan hazard of a cached prepared statement. ANALYZE executes the
    // real statement, but every EXPLAIN is rolled back.
    for statement in [
        SELECT_KNOWN_EVENT_SQL,
        SOFT_DELETE_KNOWN_EVENT_SQL,
        PURGE_KNOWN_EVENT_SQL,
    ] {
        assert!(
            statement.contains("created_at = $3"),
            "missing partition key: {statement}"
        );
        let mut tx = crate::begin_community_event_write_transaction(
            &db.pool,
            community,
            crate::observability::WriterOperation::EventWrite,
        )
        .await
        .expect("admit explain");
        sqlx::query("SET LOCAL plan_cache_mode = force_custom_plan")
            .execute(tx.conn())
            .await
            .expect("plan mode");
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "PREPARE deletion_pruning_probe (uuid, bytea, timestamptz) AS {statement}"
        )))
        .execute(tx.conn())
        .await
        .expect("prepare production SQL");
        let explain = format!(
            "EXPLAIN (ANALYZE, FORMAT JSON) EXECUTE deletion_pruning_probe ('{}', decode('{}', 'hex'), '{}')",
            community.as_uuid(), ev.id.to_hex(), timestamp(&ev).to_rfc3339()
        );
        let plan: serde_json::Value = sqlx::query_scalar(sqlx::AssertSqlSafe(explain))
            .fetch_one(tx.conn())
            .await
            .expect("production EXPLAIN");
        let mut relations = vec![];
        event_relations(&plan, &mut relations);
        assert!(!relations.is_empty(), "no event access: {plan}");
        assert!(
            relations.iter().all(|name| name == "events_p2026_01"),
            "unrelated monthly partitions: {plan}"
        );
        sqlx::query("DEALLOCATE deletion_pruning_probe")
            .execute(tx.conn())
            .await
            .expect("deallocate probe");
        tx.rollback().await.expect("rollback explain");
    }
}

/// Monthly `events` partitions (not their indexes) this backend holds a
/// relation lock on right now.
async fn locked_event_partitions(tx: &mut AdmittedTx) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT DISTINCT c.relname::text FROM pg_locks l \
         JOIN pg_class c ON c.oid = l.relation \
         WHERE l.locktype = 'relation' AND l.pid = pg_backend_pid() \
         AND c.relkind = 'r' AND c.relname LIKE 'events\\_p%' ORDER BY 1",
    )
    .fetch_all(tx.conn())
    .await
    .expect("observe partition locks")
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn production_known_target_deletion_locks_one_partition() {
    let (db, community, channel) = fixture().await;
    let slot = format!("read-state:{}", "b".repeat(32));
    let ev = event(
        30078,
        JAN,
        vec![
            Tag::parse(["d", &slot]).expect("d"),
            Tag::parse(["t", "read-state"]).expect("t"),
        ],
    );
    seed(&db, community, &ev, channel).await;
    // The assertion is only meaningful if other partitions exist to be locked.
    let partitions: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pg_inherits WHERE inhparent = 'events'::regclass")
            .fetch_one(&db.pool)
            .await
            .expect("partition count");
    assert!(partitions > 1, "need several partitions, got {partitions}");

    // The fixture pool has one connection, so every round reuses its
    // statement cache. A persistent prepared statement is custom-planned for
    // its first five executions and may then flip to a generic plan, which
    // locks every partition during plan validation; the production statements
    // are unnamed and must lock only the target month in every round.
    for round in 0..10 {
        let mut tx = crate::begin_community_event_write_transaction(
            &db.pool,
            community,
            crate::observability::WriterOperation::EventWrite,
        )
        .await
        .expect("admit deletion");
        // Known-target select, purge and soft delete all run on the production
        // path (read-state events are retention-free).
        let deleted = soft_delete_event_and_update_thread_in_tx(
            &mut tx,
            ev.id.as_bytes(),
            Some(timestamp(&ev)),
            None,
            None,
        )
        .await
        .expect("production deletion");
        assert!(deleted, "round {round}");
        let locked = locked_event_partitions(&mut tx).await;
        // Roll back so the next round deletes the same row again.
        tx.rollback().await.expect("rollback");
        assert_eq!(
            locked,
            ["events_p2026_01"],
            "round {round}: known-target deletion must lock only the target month"
        );
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn known_canvas_target_uses_canonical_replacement_lock() {
    let (db, community, channel) = fixture().await;
    let canvas = event(KIND_CANVAS as u16, JAN, vec![]);
    seed(&db, community, &canvas, channel).await;
    // The fixture's single connection identifies the backend that will delete.
    let deleter_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&db.pool)
        .await
        .expect("deleter pid");
    let observer = PgPool::connect(&crate::test_support::database_url())
        .await
        .expect("lock pool");
    let mut holder = observer.begin().await.expect("holder transaction");
    let key = crate::store::replaceable::event_replacement_lock_key(
        community,
        KIND_CANVAS as i32,
        &[],
        Some(channel.as_bytes().as_slice()),
    );
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(key)
        .execute(&mut *holder)
        .await
        .expect("hold replacement key");
    let delete_db = Db::from_pool(db.pool.clone());
    let id = canvas.id.to_bytes();
    let delete = tokio::spawn(async move {
        // No channel: a channel event this old is below the floor.
        let deletion = event(5, MAY, vec![]);
        delete_db
            .insert_deletion_event(community, &deletion, None, None, &[target(&canvas)])
            .await
    });
    // Observe an actual advisory-lock wait, not merely a task that hasn't yet
    // been scheduled. Removing the canonical lock makes this test fail.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_locks WHERE pid=$1 AND locktype='advisory' AND NOT granted)")
                .bind(deleter_pid).fetch_one(&mut *holder).await.expect("observe lock wait");
            if waiting { break; }
            assert!(!delete.is_finished(), "known canvas deletion bypassed replacement lock");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.expect("deletion must wait on replacement lock");
    holder.rollback().await.expect("release replacement key");
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), delete)
        .await
        .expect("delete completes")
        .expect("join")
        .expect("delete");
    assert_eq!(result.2, [true]);
    assert_eq!(counts(&db, community, &id).await.1, 0);
}
