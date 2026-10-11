//! PostgreSQL contract for the event follow-up producers (push match
//! enqueue and channel TTL refresh), on the desired and migration schemas,
//! with the follow-up triggers present (dual) and dropped (app-only).
use crate::replaceable::{ParameterizedReplacePrecondition, ParameterizedReplaceStatus};
use crate::{event, migration, push, AdmittedTx, Db, DbConfig, DbError};
use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use sqlx::PgPool;
use std::time::Duration;
use tokio::time::timeout;
use uuid::Uuid;

async fn connect(migration_schema: bool) -> Db {
    let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .expect("Postgres wrapper must provide an isolated database URL");
    let expected_mode = if migration_schema {
        "migration"
    } else {
        "desired"
    };
    assert_eq!(
        std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref(),
        Ok(expected_mode)
    );
    let config = DbConfig {
        push_enabled: true,
        database_url,
        ..DbConfig::default()
    };
    let db = Db::new(&config)
        .await
        .expect("connect to isolated database");
    if migration_schema {
        migration::run_migrations(db.pool())
            .await
            .expect("apply embedded migrations to empty database");
    }
    db
}

async fn create_community(pool: &PgPool) -> CommunityId {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(id)
        .bind(format!("follow-up-{}.example", id.simple()))
        .execute(pool)
        .await
        .expect("create disposable community");
    CommunityId::from_uuid(id)
}

async fn channel_with_ttl(
    pool: &PgPool,
    community: CommunityId,
    author: &[u8],
    ttl_seconds: Option<i32>,
) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO channels \
         (id, community_id, name, created_by, ttl_seconds, ttl_deadline) \
         VALUES ($1, $2, $3, $4, $5, \
             CASE WHEN $5::INT IS NULL THEN NULL \
                  ELSE clock_timestamp() - interval '1 day' END)",
    )
    .bind(id)
    .bind(community.as_uuid())
    .bind(format!("follow-up-{}", id.simple()))
    .bind(author)
    .bind(ttl_seconds)
    .execute(pool)
    .await
    .expect("create disposable channel");
    id
}

async fn create_channel(pool: &PgPool, community: CommunityId, author: &[u8]) -> Uuid {
    channel_with_ttl(pool, community, author, Some(60)).await
}

fn signed_event(keys: &Keys, kind: u16, content: &str) -> Event {
    EventBuilder::new(Kind::Custom(kind), content)
        .sign_with_keys(keys)
        .expect("sign test event")
}

async fn match_count(db: &Db, community: CommunityId, event: &Event) -> i64 {
    if let Some(producer) = &db.push_enqueue {
        producer.flush().await;
    }
    let pool = db.pool();
    sqlx::query_scalar(
        "SELECT count(*) FROM push_match_queue WHERE community_id=$1 AND event_id=$2",
    )
    .bind(community.as_uuid())
    .bind(event.id.as_bytes().as_slice())
    .fetch_one(pool)
    .await
    .expect("count push match jobs")
}

async fn event_count(pool: &PgPool, community: CommunityId, event: &Event) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid())
        .bind(event.id.as_bytes().as_slice())
        .fetch_one(pool)
        .await
        .expect("count stored events")
}

async fn deadline(pool: &PgPool, community: CommunityId, channel: Uuid) -> DateTime<Utc> {
    sqlx::query_scalar("SELECT ttl_deadline FROM channels WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid())
        .bind(channel)
        .fetch_one(pool)
        .await
        .expect("read channel deadline")
}

async fn optional_deadline(
    pool: &PgPool,
    community: CommunityId,
    channel: Uuid,
) -> Option<DateTime<Utc>> {
    sqlx::query_scalar("SELECT ttl_deadline FROM channels WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid())
        .bind(channel)
        .fetch_one(pool)
        .await
        .expect("read optional channel deadline")
}

async fn reset_deadline(pool: &PgPool, community: CommunityId, channel: Uuid) -> DateTime<Utc> {
    sqlx::query_scalar(
        "UPDATE channels SET ttl_deadline=clock_timestamp()-interval '1 day' \
         WHERE community_id=$1 AND id=$2 RETURNING ttl_deadline",
    )
    .bind(community.as_uuid())
    .bind(channel)
    .fetch_one(pool)
    .await
    .expect("reset disposable channel deadline")
}

/// `buzz_db_channel_ttl_refresh_failures_total` by `reason` label.
fn ttl_refresh_failures(snapshotter: &Snapshotter) -> Vec<(String, u64)> {
    snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .filter(|(key, ..)| key.key().name() == "buzz_db_channel_ttl_refresh_failures_total")
        .map(|(key, _, _, value)| {
            let reason = key
                .key()
                .labels()
                .find(|label| label.key() == "reason")
                .map(|label| label.value().to_owned())
                .unwrap_or_default();
            let DebugValue::Counter(count) = value else {
                panic!("TTL refresh failures must be a counter");
            };
            (reason, count)
        })
        .collect()
}

async fn begin_caller_owned_event_transaction(db: &Db, community: CommunityId) -> AdmittedTx {
    db.begin_event_write_transaction(community)
        .await
        .expect("begin admitted event transaction")
}

fn assert_sqlstate(error: &DbError, expected: &str) {
    let DbError::Sqlx(sqlx::Error::Database(database_error)) = error else {
        panic!("expected a Postgres database error, got {error:?}");
    };
    assert_eq!(database_error.code().as_deref(), Some(expected));
}

async fn activate_lease(
    pool: &PgPool,
    community: CommunityId,
    keys: &Keys,
    marker: u8,
    expires_at: i64,
) {
    let source_event_id = [marker; 32];
    let endpoint_hash = [marker.wrapping_add(64); 32];
    let subscriptions = serde_json::json!([]);
    assert_eq!(
        push::replace_active_lease(
            pool,
            community,
            &keys.public_key().to_bytes(),
            "follow-up-installation",
            push::LeaseVersion {
                source_event_id: &source_event_id,
                source_created_at: 1,
                generation: 1,
                expires_at,
            },
            push::ActiveLease {
                endpoint_hash: &endpoint_hash,
                endpoint_grant: "test-grant",
                max_class: "default",
                subscriptions: &subscriptions,
            },
        )
        .await
        .expect("activate push lease"),
        push::ReplaceLeaseOutcome::Accepted
    );
}

/// Which producers are live. `Dual` is production during the migration
/// window: the app follow-ups plus the 0023/0024 triggers. `AppOnly` drops
/// both triggers in this disposable database, as the retirement migration
/// will.
#[derive(Clone, Copy, Debug)]
enum Arm {
    Dual,
    AppOnly,
}

async fn apply_arm(pool: &PgPool, arm: Arm) {
    let triggers: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_trigger \
         WHERE tgrelid = 'events'::regclass \
           AND tgname IN ('events_enqueue_push_match', 'events_refresh_channel_ttl')",
    )
    .fetch_one(pool)
    .await
    .expect("count follow-up triggers");
    assert_eq!(triggers, 2, "both follow-up triggers exist before the arm");
    if let Arm::AppOnly = arm {
        for statement in [
            "DROP TRIGGER events_enqueue_push_match ON events",
            "DROP TRIGGER events_refresh_channel_ttl ON events",
        ] {
            sqlx::query(statement)
                .execute(pool)
                .await
                .expect("drop follow-up trigger in disposable database");
        }
        let remaining: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_trigger \
             WHERE tgname IN ('events_enqueue_push_match', 'events_refresh_channel_ttl')",
        )
        .fetch_one(pool)
        .await
        .expect("count remaining follow-up triggers, including partition clones");
        assert_eq!(
            remaining, 0,
            "app-only arm must leave no trigger on any partition"
        );
        sqlx::query("DROP FUNCTION enqueue_push_match_job()")
            .execute(pool)
            .await
            .unwrap();
        sqlx::raw_sql(include_str!(
            "../../../../migrations/0059_push_rollback_gate.sql"
        ))
        .execute(pool)
        .await
        .unwrap();
        let absent: bool =
            sqlx::query_scalar("SELECT to_regprocedure('enqueue_push_match_job()') IS NULL")
                .fetch_one(pool)
                .await
                .unwrap();
        assert!(
            absent,
            "rollback migration must not resurrect a retired function"
        );
    }
}

async fn assert_contract(migration_schema: bool, arm: Arm) {
    let db = connect(migration_schema).await;
    let pool = db.pool();
    apply_arm(pool, arm).await;
    let keys = Keys::generate();
    let community = create_community(pool).await;
    let channel = create_channel(pool, community, &keys.public_key().to_bytes()).await;
    let now = Utc::now().timestamp();
    activate_lease(pool, community, &keys, 11, now + 3600).await;

    assert_rollback(&db, community, channel, &keys).await;
    assert_async_isolation(&db, community, &keys).await;

    for kind in [9, 40002, 45001, 45003] {
        let deadline_before_first_insert = reset_deadline(pool, community, channel).await;
        let message = signed_event(&keys, kind, &format!("allowlisted-{kind}"));
        assert!(
            db.insert_event(community, &message, Some(channel))
                .await
                .expect("insert allowlisted event")
                .1
        );
        assert_eq!(event_count(pool, community, &message).await, 1);
        assert_eq!(match_count(&db, community, &message).await, 1);
        let deadline_after_first_insert = deadline(pool, community, channel).await;
        assert!(
            deadline_after_first_insert > deadline_before_first_insert,
            "first insertion of kind {kind} must refresh the channel TTL"
        );
        assert!(
            !db.insert_event(community, &message, Some(channel))
                .await
                .expect("deduplicate event")
                .1
        );
        assert_eq!(match_count(&db, community, &message).await, 1);
        assert_eq!(
            deadline(pool, community, channel).await,
            deadline_after_first_insert,
            "replaying kind {kind} must not refresh the channel TTL"
        );
    }

    let reaction = signed_event(&keys, 7, "not-push-eligible");
    assert!(
        db.insert_event(community, &reaction, Some(channel))
            .await
            .expect("insert reaction")
            .1
    );
    assert_eq!(match_count(&db, community, &reaction).await, 0);

    let no_lease_community = create_community(pool).await;
    let no_lease_message = signed_event(&keys, 9, "no-lease");
    assert!(
        db.insert_event(no_lease_community, &no_lease_message, None)
            .await
            .expect("insert without lease")
            .1
    );
    assert_eq!(
        match_count(&db, no_lease_community, &no_lease_message).await,
        0
    );

    for (marker, state) in [(21_u8, "expired"), (22, "inactive"), (23, "disabled")] {
        let predicate_community = create_community(pool).await;
        activate_lease(
            pool,
            predicate_community,
            &keys,
            marker,
            if state == "expired" {
                now - 1
            } else {
                now + 3600
            },
        )
        .await;
        if state == "inactive" {
            let source_event_id = [marker.wrapping_add(32); 32];
            assert_eq!(
                push::revoke_lease(
                    pool,
                    predicate_community,
                    &keys.public_key().to_bytes(),
                    "follow-up-installation",
                    push::LeaseVersion {
                        source_event_id: &source_event_id,
                        source_created_at: 2,
                        generation: 2,
                        expires_at: now + 3600,
                    },
                )
                .await
                .expect("revoke disposable push lease"),
                push::ReplaceLeaseOutcome::Accepted
            );
        } else if state == "disabled" {
            sqlx::query("UPDATE push_leases SET endpoint_enabled=false WHERE community_id=$1")
                .bind(predicate_community.as_uuid())
                .execute(pool)
                .await
                .expect("disable disposable push endpoint");
        }
        let message = signed_event(&keys, 9, &format!("{state}-lease"));
        assert!(
            db.insert_event(predicate_community, &message, None)
                .await
                .expect("insert event for ineligible lease")
                .1
        );
        assert_eq!(event_count(pool, predicate_community, &message).await, 1);
        assert_eq!(match_count(&db, predicate_community, &message).await, 0);
    }

    let mut push_gate_holder = pool.begin().await.expect("begin push-gate lock holder");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("buzz_push_gate:{}", community.as_uuid()))
        .execute(&mut *push_gate_holder)
        .await
        .expect("hold exclusive push-gate lock");
    let gated = signed_event(&keys, 9, "push-gate-timeout");
    let mut gated_tx = begin_caller_owned_event_transaction(&db, community).await;
    sqlx::query("SET LOCAL lock_timeout = '100ms'")
        .execute(gated_tx.conn())
        .await
        .expect("bound push-gate lock wait");
    timeout(Duration::from_secs(5), async {
        event::insert_event_in_transaction(&mut gated_tx, &gated, None)
            .await
            .unwrap();
        gated_tx.commit().await.unwrap();
    })
    .await
    .expect("push contention must not delay message commit");
    assert_eq!(event_count(pool, community, &gated).await, 1);
    assert_eq!(match_count(&db, community, &gated).await, 0);

    let ungated = signed_event(&keys, 7, "push-gate-non-eligible-control");
    let mut ungated_tx = begin_caller_owned_event_transaction(&db, community).await;
    sqlx::query("SET LOCAL lock_timeout = '100ms'")
        .execute(ungated_tx.conn())
        .await
        .expect("bound unrelated lock wait");
    assert!(
        event::insert_event_in_transaction(&mut ungated_tx, &ungated, None)
            .await
            .expect("non-push event must not use the push gate")
            .1
    );
    ungated_tx
        .commit()
        .await
        .expect("commit non-push control while gate held");
    assert_eq!(event_count(pool, community, &ungated).await, 1);
    assert_eq!(match_count(&db, community, &ungated).await, 0);
    push_gate_holder
        .rollback()
        .await
        .expect("release exclusive push-gate lock");
    let gated_control = signed_event(&keys, 9, "push-gate-released-control");
    assert!(
        db.insert_event(community, &gated_control, None)
            .await
            .expect("insert after push gate release")
            .1
    );
    assert_eq!(event_count(pool, community, &gated_control).await, 1);
    assert_eq!(match_count(&db, community, &gated_control).await, 1);

    let rollback_deadline = reset_deadline(pool, community, channel).await;
    let rolled_back = signed_event(&keys, 9, "rolled-back");
    let mut rollback_tx = begin_caller_owned_event_transaction(&db, community).await;
    assert!(
        event::insert_event_in_transaction(&mut rollback_tx, &rolled_back, Some(channel))
            .await
            .expect("insert before rollback")
            .1
    );
    let in_transaction: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM push_match_queue WHERE community_id=$1 AND event_id=$2",
    )
    .bind(community.as_uuid())
    .bind(rolled_back.id.as_bytes().as_slice())
    .fetch_one(rollback_tx.conn())
    .await
    .expect("read uncommitted follow-up");
    assert_eq!(in_transaction, 0, "push work starts only after commit");
    rollback_tx
        .rollback()
        .await
        .expect("rollback event and follow-up");
    assert_eq!(event_count(pool, community, &rolled_back).await, 0);
    assert_eq!(match_count(&db, community, &rolled_back).await, 0);
    assert_eq!(deadline(pool, community, channel).await, rollback_deadline);

    let before_commit = reset_deadline(pool, community, channel).await;
    let committed = signed_event(&keys, 9, "deferred-ttl");
    let mut commit_tx = begin_caller_owned_event_transaction(&db, community).await;
    assert!(
        event::insert_event_in_transaction(&mut commit_tx, &committed, Some(channel))
            .await
            .expect("insert before commit")
            .1
    );
    let deadline_before_commit: DateTime<Utc> =
        sqlx::query_scalar("SELECT ttl_deadline FROM channels WHERE community_id=$1 AND id=$2")
            .bind(community.as_uuid())
            .bind(channel)
            .fetch_one(commit_tx.conn())
            .await
            .expect("read deadline inside event transaction");
    assert_eq!(deadline_before_commit, before_commit);
    let database_time_before_commit: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(commit_tx.conn())
        .await
        .expect("sample database clock immediately before commit");
    commit_tx
        .commit()
        .await
        .expect("commit deferred TTL refresh");
    assert!(
        deadline(pool, community, channel).await
            >= database_time_before_commit + chrono::Duration::seconds(60),
        "the deferred trigger must refresh TTL at commit"
    );
    assert_eq!(match_count(&db, community, &committed).await, 1);

    let before_skips = reset_deadline(pool, community, channel).await;
    let channel_create = signed_event(&keys, 9007, "channel-create");
    assert!(
        db.insert_event(community, &channel_create, Some(channel))
            .await
            .expect("insert kind 9007")
            .1
    );
    assert_eq!(deadline(pool, community, channel).await, before_skips);
    assert_eq!(match_count(&db, community, &channel_create).await, 0);
    let channelless = signed_event(&keys, 9, "channel-null");
    assert!(
        db.insert_event(community, &channelless, None)
            .await
            .expect("insert channel-less event")
            .1
    );
    assert_eq!(match_count(&db, community, &channelless).await, 1);

    let no_ttl_channel =
        channel_with_ttl(pool, community, &keys.public_key().to_bytes(), None).await;
    let no_ttl_message = signed_event(&keys, 7, "ttl-null");
    assert!(
        db.insert_event(community, &no_ttl_message, Some(no_ttl_channel))
            .await
            .expect("insert into channel without TTL")
            .1
    );
    assert_eq!(
        optional_deadline(pool, community, no_ttl_channel).await,
        None
    );

    for (column, marker) in [("archived_at", "archived"), ("deleted_at", "deleted")] {
        let inactive_channel = create_channel(pool, community, &keys.public_key().to_bytes()).await;
        let inactive_deadline = deadline(pool, community, inactive_channel).await;
        let update = match column {
            "archived_at" => {
                "UPDATE channels SET archived_at=clock_timestamp() \
                 WHERE community_id=$1 AND id=$2"
            }
            "deleted_at" => {
                "UPDATE channels SET deleted_at=clock_timestamp() \
                 WHERE community_id=$1 AND id=$2"
            }
            _ => unreachable!("fixed test column"),
        };
        sqlx::query(update)
            .bind(community.as_uuid())
            .bind(inactive_channel)
            .execute(pool)
            .await
            .expect("mark disposable channel inactive");
        let message = signed_event(&keys, 7, &format!("{marker}-channel"));
        assert!(
            db.insert_event(community, &message, Some(inactive_channel))
                .await
                .expect("insert against inactive disposable channel")
                .1
        );
        assert_eq!(event_count(pool, community, &message).await, 1);
        assert_eq!(
            deadline(pool, community, inactive_channel).await,
            inactive_deadline
        );
    }

    let before_lock_timeout = reset_deadline(pool, community, channel).await;
    let mut lock_holder = pool.begin().await.expect("begin lock holder");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!(
            "buzz_channel_ttl:{}:{channel}",
            community.as_uuid()
        ))
        .execute(&mut *lock_holder)
        .await
        .expect("hold exclusive TTL lock");
    let locked_event = signed_event(&keys, 9, "ttl-lock-timeout");
    let mut locked_tx = begin_caller_owned_event_transaction(&db, community).await;
    sqlx::query("SET LOCAL lock_timeout = '100ms'")
        .execute(locked_tx.conn())
        .await
        .expect("bound TTL lock wait");
    assert!(
        event::insert_event_in_transaction(&mut locked_tx, &locked_event, Some(channel))
            .await
            .expect("insert while TTL lock held")
            .1
    );
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    {
        let _local = metrics::set_default_local_recorder(&recorder);
        timeout(Duration::from_secs(5), locked_tx.commit())
            .await
            .expect("commit must not hang on TTL lock")
            .expect("TTL warning must not abort event commit");
    }
    assert_eq!(
        ttl_refresh_failures(&snapshotter),
        [("lock_timeout".to_owned(), 1)],
        "a swallowed TTL refresh failure must be counted once, by reason"
    );

    // A cancelled statement is not swallowed: plpgsql `WHEN OTHERS` never
    // caught 57014, so the event is rejected. Both arms discriminate: if
    // the app swallowed the cancel, the app-only arm would commit the
    // event, and in the dual arm the trigger's own wait at COMMIT is not
    // cut short by the 200ms statement timeout, so the commit outlives the
    // 5s guard.
    let cancelled_event = signed_event(&keys, 9, "ttl-statement-cancel");
    let mut cancelled_tx = begin_caller_owned_event_transaction(&db, community).await;
    sqlx::query("SET LOCAL statement_timeout = '200ms'")
        .execute(cancelled_tx.conn())
        .await
        .expect("bound TTL statement");
    assert!(
        event::insert_event_in_transaction(&mut cancelled_tx, &cancelled_event, Some(channel))
            .await
            .expect("insert while TTL lock held")
            .1
    );
    let cancel_error = timeout(Duration::from_secs(5), cancelled_tx.commit())
        .await
        .expect("commit must not hang on TTL lock")
        .expect_err("a cancelled TTL refresh must reject the event");
    assert_sqlstate(&cancel_error, "57014");
    assert_eq!(event_count(pool, community, &cancelled_event).await, 0);
    assert_eq!(match_count(&db, community, &cancelled_event).await, 0);
    assert_eq!(
        deadline(pool, community, channel).await,
        before_lock_timeout
    );
    lock_holder
        .rollback()
        .await
        .expect("release exclusive TTL lock");
    assert_eq!(event_count(pool, community, &locked_event).await, 1);
    assert_eq!(match_count(&db, community, &locked_event).await, 1);

    assert_entry_points(&db, community, channel, &keys).await;
}

/// Every production writer that can carry a channel runs both follow-ups,
/// not just `insert_event`. In the app-only arm this is what catches a
/// writer that skips the hook.
async fn assert_entry_points(db: &Db, community: CommunityId, channel: Uuid, keys: &Keys) {
    let pool = db.pool();

    // Thread-metadata writer (relay ingest, reactions, serving-lease writes).
    let before = reset_deadline(pool, community, channel).await;
    let threaded = signed_event(keys, 9, "thread-metadata-writer");
    assert!(
        db.insert_event_with_thread_metadata(community, &threaded, Some(channel), None)
            .await
            .expect("insert through the thread-metadata writer")
            .1
    );
    assert_eq!(match_count(db, community, &threaded).await, 1);
    assert!(deadline(pool, community, channel).await > before);

    // Internal/external-effect completion uses the same post-commit producer.
    let lease = db
        .deletion_store()
        .acquire_serving_write_lease(community, "push-contract", "test", Duration::from_secs(60))
        .await
        .unwrap();
    let guarded = signed_event(keys, 40002, "serving-lease-writer");
    db.insert_event_with_serving_write_guard(&lease, &guarded, Some(channel))
        .await
        .unwrap();
    assert_eq!(match_count(db, community, &guarded).await, 1);
    db.deletion_store()
        .release_serving_write_lease(&lease)
        .await
        .unwrap();

    // Replaceable writer (NIP-16 and NIP-29 discovery state).
    let before = reset_deadline(pool, community, channel).await;
    let replaceable = signed_event(keys, 39000, "replaceable-writer");
    assert!(
        db.replace_addressable_event(community, &replaceable, Some(channel))
            .await
            .expect("insert through the replaceable writer")
            .1
    );
    assert_eq!(match_count(db, community, &replaceable).await, 0);
    assert!(deadline(pool, community, channel).await > before);

    // Parameterized writer: the insert runs inside a savepoint, and the
    // TTL refresh must follow only an insert that survives it.
    let d_tag = format!("follow-up-{}", Uuid::new_v4().simple());
    let parameterized = EventBuilder::new(Kind::Custom(30023), "parameterized-writer")
        .tags([Tag::identifier(d_tag.clone())])
        .sign_with_keys(keys)
        .expect("sign parameterized event");
    let before = reset_deadline(pool, community, channel).await;
    let mut tx = begin_caller_owned_event_transaction(db, community).await;
    let result = db
        .replace_parameterized_event_in_transaction(
            &mut tx,
            &parameterized,
            &d_tag,
            Some(channel),
            ParameterizedReplacePrecondition::Unconditional,
        )
        .await
        .expect("insert through the parameterized writer");
    assert_eq!(result.status, ParameterizedReplaceStatus::Inserted);
    tx.commit().await.expect("commit parameterized insert");
    assert!(deadline(pool, community, channel).await > before);

    let before = reset_deadline(pool, community, channel).await;
    let mut tx = begin_caller_owned_event_transaction(db, community).await;
    let replay = db
        .replace_parameterized_event_in_transaction(
            &mut tx,
            &parameterized,
            &d_tag,
            Some(channel),
            ParameterizedReplacePrecondition::Unconditional,
        )
        .await
        .expect("replay through the parameterized writer");
    assert_eq!(replay.status, ParameterizedReplaceStatus::Duplicate);
    tx.commit().await.expect("commit parameterized replay");
    assert_eq!(
        deadline(pool, community, channel).await,
        before,
        "a replay rolled back to its savepoint must not refresh the channel TTL"
    );

    // Member-snapshot writer (kind 39002 on the roster-locked transaction).
    let relay_keys = Keys::generate();
    let before = reset_deadline(pool, community, channel).await;
    let mut snapshot = db
        .lock_member_snapshot(community, channel, &relay_keys.public_key().to_bytes())
        .await
        .expect("lock member snapshot");
    let roster = EventBuilder::new(Kind::Custom(39002), "")
        .tags([Tag::identifier(channel.to_string())])
        .sign_with_keys(&relay_keys)
        .expect("sign member snapshot");
    assert!(
        snapshot
            .replace_member_event(&roster)
            .await
            .expect("replace member snapshot")
            .1
    );
    snapshot.release().await.expect("commit member snapshot");
    assert!(deadline(pool, community, channel).await > before);

    // One transaction, two channels, one of them twice: each channel is
    // refreshed once, at commit.
    let second = create_channel(pool, community, &keys.public_key().to_bytes()).await;
    let before_first = reset_deadline(pool, community, channel).await;
    let before_second = reset_deadline(pool, community, second).await;
    let mut tx = begin_caller_owned_event_transaction(db, community).await;
    for (target, content) in [
        (channel, "multi-a"),
        (second, "multi-b"),
        (channel, "multi-c"),
    ] {
        let message = signed_event(keys, 7, content);
        assert!(
            event::insert_event_in_transaction(&mut tx, &message, Some(target))
                .await
                .expect("insert multi-channel event")
                .1
        );
    }
    let database_time_before_commit: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(tx.conn())
        .await
        .expect("sample database clock before multi-channel commit");
    tx.commit().await.expect("commit multi-channel transaction");
    for (target, before) in [(channel, before_first), (second, before_second)] {
        let after = deadline(pool, community, target).await;
        assert!(after > before);
        assert!(after >= database_time_before_commit + chrono::Duration::seconds(60));
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn desired_event_follow_up_contract_dual() {
    assert_contract(false, Arm::Dual).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn desired_event_follow_up_contract_app_only() {
    assert_contract(false, Arm::AppOnly).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_event_follow_up_contract_dual() {
    assert_contract(true, Arm::Dual).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_event_follow_up_contract_app_only() {
    assert_contract(true, Arm::AppOnly).await;
}

// Runs with both schema sources and with/without the overlap trigger.
async fn assert_rollback(db: &Db, community: CommunityId, channel: Uuid, keys: &Keys) {
    let pool = db.pool();
    let retained = signed_event(keys, 9, "queued-before-rollback");
    db.insert_event(community, &retained, Some(channel))
        .await
        .unwrap();
    assert_eq!(match_count(db, community, &retained).await, 1);
    let lease_before: serde_json::Value =
        sqlx::query_scalar("SELECT to_jsonb(p) FROM push_leases p WHERE community_id=$1")
            .bind(community.as_uuid())
            .fetch_one(pool)
            .await
            .unwrap();

    // Include pending, claimed, and completed delivery records: rollback is
    // not a queue cleanup or a retention operation.
    for (marker, state) in [(81_u8, "pending"), (82, "sending"), (83, "delivered")] {
        sqlx::query(
            "INSERT INTO push_wake_outbox (community_id, author, installation_id, \
             lease_generation, endpoint_hash, event_id, class, expires_at, state, \
             claim_id, lease_until) \
             SELECT community_id, author, installation_id, generation, endpoint_hash, \
             $2, 'default', expires_at, $3, gen_random_uuid(), now() + interval '30 seconds' \
             FROM push_leases WHERE community_id=$1",
        )
        .bind(community.as_uuid())
        .bind(vec![marker; 32])
        .bind(state)
        .execute(pool)
        .await
        .unwrap();
    }
    let wakes_before: serde_json::Value = sqlx::query_scalar(
        "SELECT jsonb_agg(to_jsonb(w) ORDER BY id) FROM push_wake_outbox w WHERE community_id=$1",
    )
    .bind(community.as_uuid())
    .fetch_one(pool)
    .await
    .unwrap();

    let disabled = Db::new(&DbConfig {
        database_url: std::env::var("BUZZ_TEST_DATABASE_URL").unwrap(),
        push_enabled: false,
        max_connections: 1,
        min_connections: 0,
        lock_timeout_ms: 100,
        ..DbConfig::default()
    })
    .await
    .unwrap();
    let mut holder = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(push::push_gate_lock_key(community))
        .execute(&mut *holder)
        .await
        .unwrap();
    let mut disabled_events = Vec::new();
    for kind in [9, 40002, 45001, 45003] {
        // Force a fresh physical connection each time, proving after_connect
        // reapplies the flag instead of relying on a single startup session.
        disabled
            .pool()
            .acquire()
            .await
            .unwrap()
            .close()
            .await
            .unwrap();
        let message = signed_event(keys, kind, &format!("disabled-{kind}"));
        timeout(
            Duration::from_secs(5),
            disabled.insert_event(community, &message, Some(channel)),
        )
        .await
        .expect("disabled write must not wait for the push gate")
        .unwrap();
        assert_eq!(event_count(pool, community, &message).await, 1);
        assert_eq!(match_count(db, community, &message).await, 0);
        disabled_events.push(message);
    }
    holder.rollback().await.unwrap();
    assert_eq!(match_count(db, community, &retained).await, 1);
    let lease_after: serde_json::Value =
        sqlx::query_scalar("SELECT to_jsonb(p) FROM push_leases p WHERE community_id=$1")
            .bind(community.as_uuid())
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(
        lease_before, lease_after,
        "rollback preserves leases and subscriptions"
    );

    let wakes_after: serde_json::Value = sqlx::query_scalar(
        "SELECT jsonb_agg(to_jsonb(w) ORDER BY id) FROM push_wake_outbox w WHERE community_id=$1",
    )
    .bind(community.as_uuid())
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(
        wakes_before, wakes_after,
        "rollback preserves all delivery records"
    );

    // An old connection with no GUC retains the legacy producer behavior.
    // It must be replaced before operators declare rollback complete.
    let legacy_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&std::env::var("BUZZ_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let legacy = Db::from_pool(legacy_pool);
    let message = signed_event(keys, 9, "legacy-overlap-writer");
    legacy
        .insert_event(community, &message, Some(channel))
        .await
        .unwrap();
    let trigger_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_trigger WHERE tgname='events_enqueue_push_match')",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(
        match_count(db, community, &message).await,
        i64::from(trigger_exists)
    );
    legacy.pool().close().await;

    // Reactivation is an operator decision about retained queues. Reopening
    // an enabled writer produces only new work, with no lease backfill.
    let message = signed_event(keys, 9, "new-after-reactivation");
    db.insert_event(community, &message, Some(channel))
        .await
        .unwrap();
    assert_eq!(match_count(db, community, &message).await, 1);
    for message in disabled_events {
        assert_eq!(match_count(db, community, &message).await, 0);
    }
    disabled.pool().close().await;
}

async fn assert_async_isolation(db: &Db, community: CommunityId, keys: &Keys) {
    // A one-connection serving pool proves a stalled push job cannot monopolize
    // its only connection. The lock holder uses the independent fixture pool.
    let isolated = Db::new(&DbConfig {
        database_url: std::env::var("BUZZ_TEST_DATABASE_URL").unwrap(),
        max_connections: 1,
        min_connections: 0,
        lock_timeout_ms: 100,
        push_enabled: true,
        ..DbConfig::default()
    })
    .await
    .unwrap();
    let mut holder = db.pool().begin().await.unwrap();
    sqlx::query("LOCK TABLE push_match_queue IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *holder)
        .await
        .unwrap();
    let message = signed_event(keys, 9, "async-queue-lock");
    timeout(
        Duration::from_secs(5),
        isolated.insert_event(community, &message, None),
    )
    .await
    .expect("push table lock must not delay storage")
    .unwrap();
    assert_eq!(event_count(isolated.pool(), community, &message).await, 1);
    isolated.push_enqueue.as_ref().unwrap().flush().await;
    // The background statement failed its lock timeout; the message survived.
    holder.rollback().await.unwrap();
    assert_eq!(match_count(&isolated, community, &message).await, 0);
    assert_eq!(event_count(isolated.pool(), community, &message).await, 1);
    let healthy = signed_event(keys, 9, "async-after-error");
    isolated
        .insert_event(community, &healthy, None)
        .await
        .unwrap();
    assert_eq!(match_count(&isolated, community, &healthy).await, 1);

    // Exercise the timestamp filter with a lease timestamp explicitly newer
    // than this message. This is not a production activation-order regression:
    // registration overlapping message arrival may send or suppress a wake.
    let later = create_community(db.pool()).await;
    let mut activation = db.pool().begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(push::push_gate_lock_key(later))
        .execute(&mut *activation)
        .await
        .unwrap();
    let before_enrollment = signed_event(keys, 9, "before-async-enrollment");
    isolated
        .insert_event(later, &before_enrollment, None)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO push_leases (community_id, author, installation_id, source_event_id, \
         source_created_at, generation, active, endpoint_enabled, endpoint_hash, \
         endpoint_grant, max_class, subscriptions, expires_at, updated_at) \
         SELECT $1, author, installation_id, source_event_id, source_created_at, generation, \
         active, endpoint_enabled, endpoint_hash, endpoint_grant, max_class, \
         subscriptions, expires_at, clock_timestamp() FROM push_leases WHERE community_id=$2",
    )
    .bind(later.as_uuid())
    .bind(community.as_uuid())
    .execute(&mut *activation)
    .await
    .unwrap();
    activation.commit().await.unwrap();
    assert_eq!(match_count(&isolated, later, &before_enrollment).await, 0);
    assert_eq!(
        event_count(isolated.pool(), later, &before_enrollment).await,
        1
    );

    let mut holder = db.pool().begin().await.unwrap();
    sqlx::query("LOCK TABLE push_match_queue IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *holder)
        .await
        .unwrap();
    let cancelled = signed_event(keys, 9, "async-cancel-inflight");
    isolated
        .insert_event(community, &cancelled, None)
        .await
        .unwrap();
    timeout(Duration::from_secs(2), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                 WHERE datname=current_database() AND application_name='buzz-push-enqueue' \
                 AND wait_event_type='Lock' AND query LIKE 'INSERT INTO push_match_queue%')",
            )
            .fetch_one(db.pool())
            .await
            .unwrap();
            if waiting {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("observe actual enqueue blocked on the table");
    isolated.cancel_push_enqueue();
    timeout(Duration::from_secs(5), isolated.join_push_enqueue())
        .await
        .expect("shutdown cancels in-flight enqueue without waiting for lock timeout");
    holder.rollback().await.unwrap();
    assert_eq!(event_count(isolated.pool(), community, &cancelled).await, 1);
    let queued: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM push_match_queue WHERE community_id=$1 AND event_id=$2",
    )
    .bind(community.as_uuid())
    .bind(cancelled.id.as_bytes().as_slice())
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(queued, 0);
    let after_stop = signed_event(keys, 9, "async-message-after-stop");
    isolated
        .insert_event(community, &after_stop, None)
        .await
        .unwrap();
    assert_eq!(
        event_count(isolated.pool(), community, &after_stop).await,
        1
    );
    isolated.pool().close().await;
}
