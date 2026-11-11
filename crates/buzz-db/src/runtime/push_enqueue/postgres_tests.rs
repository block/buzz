//! End-to-end signed timestamp propagation and production enqueue SQL pruning.

use super::*;
use crate::{event, Db, DbConfig};
use nostr::{Event, EventBuilder, Keys, Kind, Timestamp};
use serde_json::Value;
use sqlx::AssertSqlSafe;

async fn fixture() -> (Db, CommunityId) {
    let db = Db::new(&DbConfig {
        database_url: crate::test_support::database_url(),
        push_enabled: true,
        ..DbConfig::default()
    })
    .await
    .expect("connect isolated database");
    let community = CommunityId::from_uuid(uuid::Uuid::new_v4());
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(community.as_uuid())
        .bind(format!("push-timestamp-{community}.example"))
        .execute(db.pool())
        .await
        .expect("create community");
    sqlx::query(
        "INSERT INTO push_leases (community_id, author, installation_id, \
         source_event_id, source_created_at, generation, active, endpoint_hash, \
         endpoint_grant, max_class, subscriptions, expires_at, updated_at) \
         VALUES ($1, $2, 'timestamp-test', $3, 1, 1, true, $4, 'grant', \
                 'default', '[]', EXTRACT(EPOCH FROM now())::bigint + 3600, \
                 now() - interval '1 hour')",
    )
    .bind(community.as_uuid())
    .bind(vec![1_u8; 32])
    .bind(vec![2_u8; 32])
    .bind(vec![3_u8; 32])
    .execute(db.pool())
    .await
    .expect("activate lease before event receipt");
    (db, community)
}

fn historical_event(keys: &Keys, kind: u16, date: &str) -> Event {
    let timestamp = DateTime::parse_from_rfc3339(date).expect("fixed test date");
    EventBuilder::new(Kind::Custom(kind), date)
        .custom_created_at(Timestamp::from(timestamp.timestamp() as u64))
        .sign_with_keys(keys)
        .expect("sign historical event")
}

fn signed_timestamp(event: &Event) -> DateTime<Utc> {
    DateTime::from_timestamp(event.created_at.as_secs() as i64, 0).expect("signed timestamp")
}

async fn queued_ids(db: &Db, community: CommunityId) -> Vec<Vec<u8>> {
    db.push_enqueue
        .as_ref()
        .expect("push enabled")
        .flush()
        .await;
    sqlx::query_scalar(
        "SELECT event_id FROM push_match_queue WHERE community_id = $1 ORDER BY event_id",
    )
    .bind(community.as_uuid())
    .fetch_all(db.pool())
    .await
    .expect("read completed enqueue jobs")
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn historical_timestamps_follow_each_staged_event_and_both_insert_seams() {
    let (db, community) = fixture().await;
    let keys = Keys::generate();
    // Deliberately not timestamp ordered: a transaction-wide timestamp or a
    // swapped association must lose at least one wake. Receipt time is today.
    let mut events = Vec::new();
    let mut tx = db.begin_event_write_transaction(community).await.unwrap();
    for (kind, date) in [
        (9, "2026-04-20T23:59:59Z"),
        (40002, "2025-09-01T00:00:00Z"),
        (45001, "2026-02-01T00:00:00Z"),
        (45003, "2026-01-31T23:59:59Z"),
    ] {
        let event = historical_event(&keys, kind, date);
        assert!(
            event::insert_event_in_transaction(&mut tx, &event, None)
                .await
                .unwrap()
                .1
        );
        events.push(event);
    }
    assert!(queued_ids(&db, community).await.is_empty(), "commit only");
    tx.commit().await.unwrap();

    let threaded = historical_event(&keys, 9, "2026-03-01T00:00:00Z");
    assert!(
        db.insert_event_with_thread_metadata(community, &threaded, None, None)
            .await
            .unwrap()
            .1
    );
    events.push(threaded);
    let mut expected: Vec<_> = events
        .iter()
        .map(|event| event.id.as_bytes().to_vec())
        .collect();
    expected.sort();
    assert_eq!(queued_ids(&db, community).await, expected);

    // Remove completed jobs so ON CONFLICT cannot hide an erroneous enqueue
    // for a duplicate/no-op insert on either production insert seam.
    sqlx::query("DELETE FROM push_match_queue WHERE community_id = $1")
        .bind(community.as_uuid())
        .execute(db.pool())
        .await
        .unwrap();
    assert!(
        !db.insert_event(community, &events[0], None)
            .await
            .unwrap()
            .1
    );
    assert!(
        !db.insert_event_with_thread_metadata(community, &events[4], None, None)
            .await
            .unwrap()
            .1
    );
    assert!(queued_ids(&db, community).await.is_empty());

    let rolled_back = historical_event(&keys, 9, "2026-05-01T00:00:00Z");
    let mut tx = db.begin_event_write_transaction(community).await.unwrap();
    assert!(
        event::insert_event_in_transaction(&mut tx, &rolled_back, None)
            .await
            .unwrap()
            .1
    );
    tx.rollback().await.unwrap();
    assert!(queued_ids(&db, community).await.is_empty());
    let dropped = historical_event(&keys, 9, "2026-06-01T00:00:00Z");
    let mut tx = db.begin_event_write_transaction(community).await.unwrap();
    event::insert_event_in_transaction(&mut tx, &dropped, None)
        .await
        .unwrap();
    drop(tx);
    assert!(queued_ids(&db, community).await.is_empty());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn enqueue_requires_exact_signed_timestamp_not_receipt_or_neighbor_month() {
    let (db, community) = fixture().await;
    let event = historical_event(&Keys::generate(), 9, "2026-02-01T00:00:00Z");
    // Store without the producer; exercise the real worker transaction below.
    event::insert_event(db.pool(), community, &event, None)
        .await
        .unwrap();
    let mut job = Job {
        community,
        event_id: event.id.as_bytes().to_vec(),
        created_at: signed_timestamp(&event) - chrono::Duration::seconds(1),
        barrier: None,
    };
    enqueue(db.pool(), &job, None).await.unwrap();
    assert!(queued_ids(&db, community).await.is_empty());
    job.created_at = Utc::now();
    enqueue(db.pool(), &job, None).await.unwrap();
    assert!(queued_ids(&db, community).await.is_empty());
    job.created_at = signed_timestamp(&event);
    enqueue(db.pool(), &job, None).await.unwrap();
    assert_eq!(queued_ids(&db, community).await, vec![job.event_id]);
}

fn executed_event_relations(plan: &Value, relations: &mut Vec<String>) {
    match plan {
        Value::Object(node) => {
            // Generic plans name Append children e_1, e_2, ... rather than
            // e. Inspect base relations, not aliases or a particular plan shape.
            if node
                .get("Actual Loops")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                > 0
            {
                if let Some(relation) = node.get("Relation Name").and_then(Value::as_str) {
                    if relation.starts_with("events_") {
                        relations.push(relation.to_owned());
                    }
                }
            }
            for child in node.values() {
                executed_event_relations(child, relations);
            }
        }
        Value::Array(nodes) => {
            for child in nodes {
                executed_event_relations(child, relations);
            }
        }
        _ => {}
    }
}

/// The custom plan the production statement receives executes one partition.
/// Executed nodes say nothing about locks: a generic plan also executes one
/// partition after locking all of them, so lock coverage is asserted below.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn production_enqueue_sql_executes_one_month_in_custom_plan() {
    let (db, community) = fixture().await;
    let event = historical_event(&Keys::generate(), 9, "2026-02-01T00:00:00Z");
    event::insert_event(db.pool(), community, &event, None)
        .await
        .unwrap();
    let mut tx = db.pool().begin().await.unwrap();
    sqlx::query("SELECT set_config('plan_cache_mode', 'force_custom_plan', true)")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(AssertSqlSafe(format!(
        "PREPARE push_pruning (uuid, bytea, timestamptz) AS {ENQUEUE_SQL}"
    )))
    .execute(&mut *tx)
    .await
    .unwrap();
    // Only generated UUID/hex and a fixed signed date enter this test SQL.
    // EXPLAIN the exact INSERT used by enqueue, not a lookalike SELECT.
    let plan: Value = sqlx::query_scalar(AssertSqlSafe(format!(
        "EXPLAIN (ANALYZE, FORMAT JSON) EXECUTE push_pruning \
         ('{community}', '\\x{}'::bytea, '{}')",
        hex::encode(event.id.as_bytes()),
        signed_timestamp(&event).to_rfc3339(),
    )))
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let mut relations = Vec::new();
    executed_event_relations(&plan, &mut relations);
    assert_eq!(relations, ["events_p2026_02"], "{plan}");
    tx.rollback().await.unwrap();
}

/// The production statement must lock one `events` partition. Locks, not
/// executed plan nodes, are what a long-lived push worker would contend on: a
/// generic plan executes one partition only after locking all of them. The
/// statement must therefore never be a cached prepared statement, which
/// Postgres may move to a generic plan after five executions.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn production_enqueue_statement_locks_only_the_signed_month_partition() {
    let (db, community) = fixture().await;
    let event = historical_event(&Keys::generate(), 9, "2026-02-01T00:00:00Z");
    event::insert_event(db.pool(), community, &event, None)
        .await
        .unwrap();
    let job = Job {
        community,
        event_id: event.id.as_bytes().to_vec(),
        created_at: signed_timestamp(&event),
        barrier: None,
    };
    let mut tx = db.pool().begin().await.unwrap();
    // Past the five-execution custom-plan window of a cached statement.
    for _ in 0..7 {
        insert_queue_row(&mut tx, &job).await.unwrap();
    }
    let cached: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_prepared_statements \
         WHERE statement LIKE 'INSERT INTO push_match_queue%'",
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert_eq!(cached, 0, "enqueue SQL must not be a cached statement");
    // Locks on a partition or any of its indexes, attributed to the partition.
    let locked: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT t.relname::text FROM pg_locks l \
         JOIN pg_class c ON c.oid = l.relation \
         LEFT JOIN pg_index i ON i.indexrelid = c.oid \
         JOIN pg_class t ON t.oid = COALESCE(i.indrelid, c.oid) \
         WHERE l.locktype = 'relation' AND l.pid = pg_backend_pid() \
           AND t.relname LIKE 'events\\_p%' ORDER BY 1",
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    assert_eq!(locked, ["events_p2026_02"]);
    tx.rollback().await.unwrap();
}
