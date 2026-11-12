use super::*;
use nostr::{EventBuilder, Keys, Kind, Timestamp};
use sqlx::{PgConnection, PgPool, Row};
use std::collections::BTreeSet;
use uuid::Uuid;

const JAN: i64 = 1_768_435_200; // 2026-01-15 UTC
const MAR: i64 = 1_773_532_800; // 2026-03-15 UTC

async fn fixture() -> (Db, CommunityId, CommunityId, Keys) {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let db = Db::from_pool(pool);
    let mut communities = Vec::new();
    for _ in 0..2 {
        communities.push(
            db.ensure_configured_community(&format!("hydrate-{}.local", Uuid::new_v4()))
                .await
                .unwrap()
                .id,
        );
    }
    (db, communities[0], communities[1], Keys::generate())
}

fn event(keys: &Keys, content: &str, seconds: i64) -> nostr::Event {
    EventBuilder::new(Kind::Custom(9), content)
        .custom_created_at(Timestamp::from(seconds as u64))
        .sign_with_keys(keys)
        .unwrap()
}

fn key(event: &nostr::Event) -> ([u8; 32], i64) {
    (event.id.to_bytes(), event.created_at.as_secs() as i64)
}

fn ids(events: &[StoredEvent]) -> BTreeSet<[u8; 32]> {
    events
        .iter()
        .map(|event| event.event.id.to_bytes())
        .collect()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn hydration_exact_pairs_across_months_and_scopes() {
    let (db, a, b, keys) = fixture().await;
    let jan = event(&keys, "January", JAN);
    let same_second = event(&keys, "January same second", JAN);
    let march = event(&keys, "March", MAR);
    let deleted = event(&keys, "deleted", JAN + 1);
    let foreign = event(&keys, "foreign", MAR);
    for event in [&jan, &same_second, &march, &deleted] {
        db.insert_event(a, event, None).await.unwrap();
    }
    // Same signed event in two communities is legal. A's tombstone must not
    // fall through to B's live copy.
    db.insert_event(b, &deleted, None).await.unwrap();
    db.insert_event(b, &foreign, None).await.unwrap();
    sqlx::query("UPDATE events SET deleted_at = now() WHERE community_id = $1 AND id = $2 AND created_at = $3")
        .bind(a.as_uuid()).bind(deleted.id.to_bytes().to_vec())
        .bind(DateTime::from_timestamp(JAN + 1, 0).unwrap())
        .execute(&db.pool).await.unwrap();

    let hits = [
        key(&march),
        key(&jan),
        key(&same_second),
        key(&jan),
        key(&deleted),
        key(&foreign),
        ([0xff; 32], JAN),
    ];
    let found = db
        .get_events_by_ids_at_routed("req_search_hydrate", a, &hits)
        .await
        .unwrap();
    assert_eq!(found.len(), 3, "duplicate inputs must not multiply rows");
    assert_eq!(
        ids(&found),
        [
            jan.id.to_bytes(),
            same_second.id.to_bytes(),
            march.id.to_bytes()
        ]
        .into()
    );
    assert_eq!(
        ids(&db
            .get_events_by_ids_at_routed(
                "profile_search_hydrate",
                b,
                &[key(&deleted), key(&foreign), key(&jan)]
            )
            .await
            .unwrap()),
        [deleted.id.to_bytes(), foreign.id.to_bytes()].into()
    );

    // Both timestamps and both IDs are real, but neither PAIR exists. This
    // fails if the production predicate becomes independent id/time IN lists.
    let crossed = [(jan.id.to_bytes(), MAR), (march.id.to_bytes(), JAN)];
    assert!(db
        .get_events_by_ids_at_routed("bridge_search_hydrate", a, &crossed)
        .await
        .unwrap()
        .is_empty());
    assert!(db
        .get_events_by_ids_at_routed("req_search_hydrate", a, &[(jan.id.to_bytes(), JAN + 1)])
        .await
        .unwrap()
        .is_empty());

    // Existing ID-only callers retain their contract.
    let raw = [jan.id.as_bytes().as_slice(), march.id.as_bytes().as_slice()];
    assert_eq!(db.get_events_by_ids(a, &raw).await.unwrap().len(), 2);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn hydration_rejects_invalid_or_oversized_batches_before_checkout() {
    // A closed pool makes any accidental database access fail. Invalid inputs
    // must still return the input error, and empty input must succeed.
    let (db, community, _, _) = fixture().await;
    db.pool.close().await;
    for seconds in [-1, i64::MIN, i64::MAX] {
        assert!(
            matches!(db.get_events_by_ids_at_routed("req_search_hydrate", community, &[([0; 32], seconds)]).await,
            Err(DbError::InvalidTimestamp(value)) if value == seconds)
        );
    }
    assert!(matches!(
        db.get_events_by_ids_at_routed(
            "req_search_hydrate",
            community,
            &vec![([0; 32], JAN); MAX_BATCH + 1]
        )
        .await,
        Err(DbError::InvalidData(_))
    ));
    assert!(db
        .get_events_by_ids_at_routed("req_search_hydrate", community, &[])
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn hydration_accepts_full_bounded_batch_and_deduplicates() {
    let (db, community, _, keys) = fixture().await;
    let event = event(&keys, "full batch", JAN);
    db.insert_event(community, &event, None).await.unwrap();
    let found = db
        .get_events_by_ids_at_routed(
            "req_search_hydrate",
            community,
            &vec![key(&event); MAX_BATCH],
        )
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].event.id, event.id);
    // Exercise the full 1001-bind SQL shape too, not just deduplication down
    // to one predicate. Other keys are intentionally absent.
    let mut distinct = Vec::with_capacity(MAX_BATCH);
    for n in 0..MAX_BATCH - 1 {
        let mut id = [0; 32];
        id[..8].copy_from_slice(&(n as u64).to_be_bytes());
        distinct.push((id, JAN));
    }
    distinct.push(key(&event));
    let found = db
        .get_events_by_ids_at_routed("req_search_hydrate", community, &distinct)
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].event.id, event.id);
}

fn executed_relations(node: &serde_json::Value, out: &mut BTreeSet<String>) {
    if node["Actual Loops"].as_u64().unwrap_or(0) > 0 {
        if let Some(name) = node["Relation Name"].as_str() {
            out.insert(name.to_owned());
        }
    }
    if let Some(children) = node["Plans"].as_array() {
        for child in children {
            executed_relations(child, out);
        }
    }
}

/// Partitions of `events` that `conn`'s open transaction holds a relation lock
/// on. Indexes are excluded (`relkind = 'r'`), the partitioned parent too.
async fn locked_event_partitions(conn: &mut PgConnection) -> BTreeSet<String> {
    sqlx::query_scalar(
        "SELECT c.relname::text FROM pg_locks l JOIN pg_class c ON c.oid = l.relation \
         WHERE l.locktype = 'relation' AND l.pid = pg_backend_pid() \
           AND c.relkind = 'r' AND c.relname LIKE 'events\\_p%'",
    )
    .fetch_all(conn)
    .await
    .unwrap()
    .into_iter()
    .collect()
}

/// Pins what the hydration lookup LOCKS, not just what it executes: the
/// partitions other than the requested months must stay unlocked.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn hydration_lookup_locks_only_the_requested_partitions() {
    let (db, community, _, keys) = fixture().await;
    let jan = event(&keys, "lock January", JAN);
    let march = event(&keys, "lock March", MAR);
    for event in [&jan, &march] {
        db.insert_event(community, event, None).await.unwrap();
    }
    // Mixed present/absent keys across both months, at the full batch size.
    let mut large = vec![key(&jan), key(&march)];
    for n in 0..MAX_BATCH - large.len() {
        let mut id = [0; 32];
        id[..8].copy_from_slice(&(n as u64).to_be_bytes());
        large.push((id, if n % 2 == 0 { JAN } else { MAR }));
    }
    let cases = [
        (vec![key(&jan)], vec!["events_p2026_01"], 1),
        (vec![key(&march)], vec!["events_p2026_03"], 1),
        (
            vec![key(&jan), key(&march)],
            vec!["events_p2026_01", "events_p2026_03"],
            2,
        ),
        (large, vec!["events_p2026_01", "events_p2026_03"], 2),
    ];
    // Run the production `lookup_on` repeatedly on ONE session. A cached
    // prepared statement is eligible to flip to a generic plan after five
    // executions, and on PG16+ a generic plan locks every partition at
    // executor startup before pruning. The unnamed statement used in
    // production is planned afresh with its parameter values every time.
    let mut conn = db.pool.acquire().await.unwrap();
    for round in 0..8 {
        for (hits, expected_locks, expected_rows) in &cases {
            let keys = validate(hits).unwrap();
            let mut tx = sqlx::Connection::begin(&mut *conn).await.unwrap();
            let found = lookup_on(&mut tx, community, &keys).await.unwrap();
            assert_eq!(found.len(), *expected_rows, "round {round}");
            let locked = locked_event_partitions(&mut tx).await;
            tx.rollback().await.unwrap();
            let expected: BTreeSet<String> = expected_locks
                .iter()
                .map(|name| (*name).to_owned())
                .collect();
            assert_eq!(
                locked,
                expected,
                "round {round}, {} keys: lookup_on locked unrelated events partitions",
                keys.len()
            );
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn hydration_production_sql_executes_only_requested_partitions() {
    let (db, community, _, keys) = fixture().await;
    let jan = event(&keys, "plan January", JAN);
    let march = event(&keys, "plan March", MAR);
    let wrong_time = event(&keys, "plan January, requested in March", JAN);
    for event in [&jan, &march, &wrong_time] {
        db.insert_event(community, event, None).await.unwrap();
    }
    // Reuse the tiny fixture for the full 1001-bind shape. Missing keys span
    // both months; the third seeded event must NOT match its requested time.
    // Independent id/time IN lists would incorrectly return that third row.
    let mut large_hits = vec![key(&jan), key(&march), (wrong_time.id.to_bytes(), MAR)];
    for n in 0..MAX_BATCH - large_hits.len() {
        let mut id = [0; 32];
        id[..8].copy_from_slice(&(n as u64).to_be_bytes());
        large_hits.push((id, if n % 2 == 0 { JAN } else { MAR }));
    }
    assert_eq!(large_hits.len(), 500);
    assert_eq!(
        large_hits.iter().copied().collect::<BTreeSet<_>>().len(),
        500
    );
    let cases = [
        (vec![key(&jan)], 1),
        (vec![key(&jan), key(&march)], 2),
        (large_hits, 2),
    ];
    let mut conn = db.pool.acquire().await.unwrap();
    // EXPLAIN ANALYZE the actual production builder, not a copied
    // approximation, as a custom plan (the plan the production statement is
    // executed with). It reports relations EXECUTED, not relations LOCKED: the
    // generic-plan lock behavior is pinned by
    // `hydration_lookup_locks_only_the_requested_partitions`.
    for (hits, expected_rows) in &cases {
        let validated = validate(hits).unwrap();
        assert_eq!(validated.len(), hits.len(), "plan keys must stay distinct");
        let query = lookup_sql(community, &validated);
        sqlx::query("SET plan_cache_mode = force_custom_plan")
            .execute(&mut *conn)
            .await
            .unwrap();
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "PREPARE hydration_plan AS {}",
            query.sql().as_str()
        )))
        .execute(&mut *conn)
        .await
        .unwrap();
        let mut arguments = vec![format!("'{}'::uuid", community.as_uuid())];
        for (id, timestamp) in &validated {
            arguments.push(format!("'{}'::timestamptz", timestamp.to_rfc3339()));
            arguments.push(format!("decode('{}', 'hex')", hex::encode(id)));
        }
        let plan: serde_json::Value = sqlx::query(sqlx::AssertSqlSafe(format!(
            "EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) EXECUTE hydration_plan({})",
            arguments.join(", ")
        )))
        .fetch_one(&mut *conn)
        .await
        .unwrap()
        .get(0);
        let mut relations = BTreeSet::new();
        executed_relations(&plan[0]["Plan"], &mut relations);
        let expected: BTreeSet<String> = if hits.len() == 1 {
            ["events_p2026_01".to_owned()].into()
        } else {
            ["events_p2026_01".to_owned(), "events_p2026_03".to_owned()].into()
        };
        assert_eq!(
            relations,
            expected,
            "{} keys: unrelated partitions executed: {plan}",
            hits.len()
        );
        assert_eq!(
            plan[0]["Plan"]["Actual Rows"].as_u64(),
            Some(*expected_rows),
            "{} keys: unexpected hydrated row count: {plan}",
            hits.len()
        );
        sqlx::query("DEALLOCATE hydration_plan")
            .execute(&mut *conn)
            .await
            .unwrap();
    }
    sqlx::query("RESET plan_cache_mode")
        .execute(&mut *conn)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn channel_create_resend_requires_exact_timestamp_and_live_community_row() {
    use crate::channel::{ChannelCreate, ChannelType, ChannelVisibility, NewChannel};
    let (db, a, b, keys) = fixture().await;
    let channel_id = Uuid::new_v4();
    let creator = keys.public_key().to_bytes();
    // Created now: the serving writer rejects a channel event older than the
    // `created_at` floor, and the first create must succeed. The malformed
    // March row below still differs from it in the partition key.
    let created_at = Timestamp::now();
    let event = EventBuilder::new(Kind::Custom(9007), "channel create")
        .custom_created_at(created_at)
        .sign_with_keys(&keys)
        .unwrap();
    let channel = || NewChannel {
        name: "exact-resend",
        channel_type: ChannelType::Stream,
        visibility: ChannelVisibility::Private,
        description: None,
        created_by: &creator,
        ttl_seconds: None,
        labels: &[],
        unlisted: false,
        members: &[],
    };
    assert!(matches!(
        db.create_channel_with_event(a, channel_id, channel(), &event)
            .await
            .unwrap(),
        ChannelCreate::Created(_)
    ));
    assert!(matches!(
        db.create_channel_with_event(a, channel_id, channel(), &event)
            .await
            .unwrap(),
        ChannelCreate::Exists { resent: true }
    ));
    // Keep a live exact-key copy in B so community scope is independently
    // falsifiable even when A's copy is deleted or has a mismatched key.
    db.insert_event(b, &event, None).await.unwrap();
    sqlx::query("UPDATE events SET deleted_at = now() WHERE community_id = $1 AND id = $2 AND created_at = $3")
        .bind(a.as_uuid()).bind(event.id.to_bytes().to_vec())
        .bind(DateTime::from_timestamp(created_at.as_secs() as i64, 0).unwrap())
        .execute(&db.pool).await.unwrap();
    assert!(matches!(
        db.create_channel_with_event(a, channel_id, channel(), &event)
            .await
            .unwrap(),
        ChannelCreate::Exists { resent: false }
    ));
    // Deliberately malformed storage: id alone matches a live row in A, but
    // its partition key does not. This catches removal of created_at equality.
    sqlx::query(
        "UPDATE events SET deleted_at = NULL, created_at = $3 WHERE community_id = $1 AND id = $2",
    )
    .bind(a.as_uuid())
    .bind(event.id.to_bytes().to_vec())
    .bind(DateTime::from_timestamp(MAR, 0).unwrap())
    .execute(&db.pool)
    .await
    .unwrap();
    assert!(matches!(
        db.create_channel_with_event(a, channel_id, channel(), &event)
            .await
            .unwrap(),
        ChannelCreate::Exists { resent: false }
    ));
    let mut invalid = event.clone();
    invalid.created_at = Timestamp::from(u64::MAX);
    assert!(matches!(
        db.create_channel_with_event(a, channel_id, channel(), &invalid)
            .await,
        Err(DbError::InvalidTimestamp(_))
    ));
}
