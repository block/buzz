//! App-floor regressions with every floor-trigger clone disabled in the
//! nextest-provided per-test database, so the DB backstop cannot mask a bypass.

use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use nostr::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

use crate::replica_fence::{
    probe_once, ReplicaFence, CREATED_AT_FLOOR_SECS, FENCE_CLOCK_MARGIN_SECS,
};
use crate::{event, AdmittedTx, Db, DbError};

async fn fixture() -> (Db, CommunityId, Uuid, Keys) {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&crate::test_support::database_url())
        .await
        .expect("isolated test database");
    sqlx::query("ALTER TABLE events DISABLE TRIGGER events_created_at_floor")
        .execute(&pool)
        .await
        .expect("disable floor trigger recursively on the partitioned table");
    let (total, enabled): (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(*) FILTER (WHERE tgenabled <> 'D') FROM pg_trigger \
         WHERE tgname = 'events_created_at_floor' \
         AND tgrelid IN (SELECT relid FROM pg_partition_tree('events'))",
    )
    .fetch_one(&pool)
    .await
    .expect("check every trigger clone");
    assert!(total > 0, "the test must disable an existing backstop");
    assert_eq!(enabled, 0, "no partition may mask a missing app check");
    let community = CommunityId::from_uuid(Uuid::new_v4());
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(community.as_uuid())
        .bind(format!("floor-{}.invalid", community.as_uuid()))
        .execute(&pool)
        .await
        .expect("community");
    let channel = Uuid::new_v4();
    let keys = Keys::generate();
    sqlx::query(
        "INSERT INTO channels (id, community_id, name, created_by) VALUES ($1,$2,'floor',$3)",
    )
    .bind(channel)
    .bind(community.as_uuid())
    .bind(keys.public_key().to_bytes().as_slice())
    .execute(&pool)
    .await
    .expect("channel");
    (Db::from_pool(pool), community, channel, keys)
}

async fn database_clock(db: &Db) -> DateTime<Utc> {
    sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(db.pool())
        .await
        .expect("database clock")
}

fn signed(keys: &Keys, kind: u16, channel: Uuid, time: DateTime<Utc>) -> Event {
    EventBuilder::new(Kind::Custom(kind), "floor-test")
        .tags([Tag::parse(["d", &channel.to_string()]).expect("coordinate")])
        .custom_created_at(Timestamp::from(time.timestamp() as u64))
        .sign_with_keys(keys)
        .expect("signed event")
}

async fn count(db: &Db, community: CommunityId, event: &Event) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid())
        .bind(event.id.as_bytes().as_slice())
        .fetch_one(db.pool())
        .await
        .expect("event count")
}

async fn begin(db: &Db, community: CommunityId) -> AdmittedTx {
    db.begin_event_write_transaction(community)
        .await
        .expect("admission")
}

fn rejected<T: std::fmt::Debug>(result: crate::Result<T>) {
    assert!(
        matches!(result, Err(DbError::CreatedAtFloorRejected)),
        "expected the typed app-floor rejection, got {result:?}"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn insert_event_floor_rejects_old_and_accepts_fresh() {
    let (db, community, channel, keys) = fixture().await;
    let now = database_clock(&db).await;
    let old = signed(
        &keys,
        9,
        channel,
        now - chrono::Duration::seconds(CREATED_AT_FLOOR_SECS + 60),
    );
    rejected(event::insert_event(db.pool(), community, &old, Some(channel)).await);
    assert_eq!(count(&db, community, &old).await, 0);
    let fresh = signed(&keys, 9, channel, now);
    assert!(
        event::insert_event(db.pool(), community, &fresh, Some(channel))
            .await
            .expect("fresh")
            .1
    );
    assert_eq!(count(&db, community, &fresh).await, 1);
    // Channel-NULL history is structurally outside covered windows.
    assert!(
        event::insert_event(db.pool(), community, &old, None)
            .await
            .expect("NULL-channel history")
            .1
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_metadata_event_floor_rejects_old_and_accepts_fresh() {
    let (db, community, channel, keys) = fixture().await;
    let now = database_clock(&db).await;
    for (time, old) in [
        (
            now - chrono::Duration::seconds(CREATED_AT_FLOOR_SECS + 60),
            true,
        ),
        (now, false),
    ] {
        let event = signed(&keys, 9, channel, time);
        let meta = event::ThreadMetadataParams {
            event_id: event.id.as_bytes().as_slice(),
            event_created_at: time,
            channel_id: channel,
            parent_event_id: None,
            parent_event_created_at: None,
            root_event_id: None,
            root_event_created_at: None,
            depth: 0,
            broadcast: false,
        };
        let mut tx = begin(&db, community).await;
        let result =
            event::insert_event_with_thread_metadata_tx(&mut tx, &event, Some(channel), Some(meta))
                .await;
        if old {
            rejected(result);
            tx.rollback().await.expect("rollback refusal");
            assert_eq!(count(&db, community, &event).await, 0);
        } else {
            assert!(result.expect("fresh").1);
            tx.commit().await.expect("commit");
            assert_eq!(count(&db, community, &event).await, 1);
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn parameterized_event_floor_rejects_old_and_accepts_fresh() {
    let (db, community, channel, keys) = fixture().await;
    let now = database_clock(&db).await;
    let old = signed(
        &keys,
        30078,
        channel,
        now - chrono::Duration::seconds(CREATED_AT_FLOOR_SECS + 60),
    );
    rejected(
        db.replace_parameterized_event(community, &old, &channel.to_string(), Some(channel))
            .await,
    );
    assert_eq!(count(&db, community, &old).await, 0);
    let fresh = signed(&keys, 30078, channel, now);
    assert!(
        db.replace_parameterized_event(community, &fresh, &channel.to_string(), Some(channel))
            .await
            .expect("fresh")
            .1
    );
    assert_eq!(count(&db, community, &fresh).await, 1);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn addressable_event_floor_rejects_old_and_accepts_fresh() {
    let (db, community, channel, keys) = fixture().await;
    let now = database_clock(&db).await;
    let old = signed(
        &keys,
        39000,
        channel,
        now - chrono::Duration::seconds(CREATED_AT_FLOOR_SECS + 60),
    );
    rejected(
        db.replace_addressable_event(community, &old, Some(channel))
            .await,
    );
    assert_eq!(count(&db, community, &old).await, 0);
    let fresh = signed(&keys, 39000, channel, now);
    assert!(
        db.replace_addressable_event(community, &fresh, Some(channel))
            .await
            .expect("fresh")
            .1
    );
    assert_eq!(count(&db, community, &fresh).await, 1);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn member_snapshot_event_floor_rejects_old_and_accepts_fresh() {
    let (db, community, channel, keys) = fixture().await;
    let now = database_clock(&db).await;
    let old = signed(
        &keys,
        39002,
        channel,
        now - chrono::Duration::seconds(CREATED_AT_FLOOR_SECS + 60),
    );
    let mut snapshot = db
        .lock_member_snapshot(community, channel, &keys.public_key().to_bytes())
        .await
        .expect("snapshot");
    rejected(snapshot.replace_member_event(&old).await);
    // A refusal must leave the held snapshot usable and write no event.
    assert_eq!(count(&db, community, &old).await, 0);
    let fresh = signed(&keys, 39002, channel, now);
    assert!(
        snapshot
            .replace_member_event(&fresh)
            .await
            .expect("fresh")
            .1
    );
    snapshot.release().await.expect("commit snapshot");
    assert_eq!(count(&db, community, &fresh).await, 1);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn cluster_global_event_floor_held_transaction_cannot_write_below_probe_wall() {
    let (db, community, channel, keys) = fixture().await;
    let mut tx = begin(&db, community).await;
    let start: DateTime<Utc> = sqlx::query_scalar("SELECT now()")
        .fetch_one(tx.conn())
        .await
        .expect("transaction start");
    let entry = probe_once(db.pool(), &ReplicaFence::new())
        .await
        .expect("heartbeat while writer is open");
    assert!(
        entry.fence_wall
            <= start - chrono::Duration::seconds(CREATED_AT_FLOOR_SECS + FENCE_CLOCK_MARGIN_SECS)
    );
    let below_wall = signed(
        &keys,
        9,
        channel,
        entry.fence_wall - chrono::Duration::seconds(1),
    );
    rejected(event::insert_event_in_transaction(&mut tx, &below_wall, Some(channel)).await);
    // Sample just after the probe; a near-floor accepted insert is above its
    // wall even when this transaction commits after the heartbeat.
    let fresh = signed(
        &keys,
        9,
        channel,
        database_clock(&db).await - chrono::Duration::seconds(CREATED_AT_FLOOR_SECS - 5),
    );
    let created_at =
        DateTime::from_timestamp(fresh.created_at.as_secs() as i64, 0).expect("timestamp");
    assert!(created_at > entry.fence_wall);
    assert!(
        event::insert_event_in_transaction(&mut tx, &fresh, Some(channel))
            .await
            .expect("near floor")
            .1
    );
    tx.commit().await.expect("commit after probe");
    assert_eq!(count(&db, community, &below_wall).await, 0);
    assert_eq!(count(&db, community, &fresh).await, 1);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn event_floor_samples_statement_clock_after_transaction_start() {
    let (db, community, channel, keys) = fixture().await;
    let mut tx = begin(&db, community).await;
    // The smallest integral event timestamp inside the floor at BEGIN.
    let secs: i64 =
        sqlx::query_scalar("SELECT ceil(extract(epoch FROM now()) - $1::double precision)::bigint")
            .bind(CREATED_AT_FLOOR_SECS as f64)
            .fetch_one(tx.conn())
            .await
            .expect("BEGIN boundary");
    let time = DateTime::from_timestamp(secs, 0).expect("timestamp");
    // Wait on the DB clock until this exact boundary expires (at most 1.1 s),
    // not an arbitrary delay and not a Rust clock. now() remains frozen inside.
    tokio::time::timeout(std::time::Duration::from_secs(3),
        sqlx::query("SELECT pg_sleep(GREATEST(0, extract(epoch FROM ($1::timestamptz + make_interval(secs => $2::double precision) - clock_timestamp()))) + 0.1)")
            .bind(time).bind(CREATED_AT_FLOOR_SECS as f64).execute(tx.conn()))
        .await.expect("bounded boundary wait").expect("database wait");
    let event = signed(&keys, 9, channel, time);
    rejected(event::insert_event_in_transaction(&mut tx, &event, Some(channel)).await);
    tx.rollback().await.expect("rollback");
    assert_eq!(count(&db, community, &event).await, 0);
}
