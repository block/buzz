use super::*;
use crate::{relay_members, thread};
use buzz_core::CommunityId;
use sqlx::{Connection, PgPool};
use uuid::Uuid;

const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1 -- local test-only credentials

async fn setup_db() -> Db {
    let database_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| TEST_DB_URL.into());
    let pool = PgPool::connect(&database_url)
        .await
        .expect("connect to test DB");
    if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() == Ok("migration") {
        crate::migration::run_migrations(&pool)
            .await
            .expect("apply migration schema");
    }
    Db::from_pool(pool)
}

#[tokio::test]
async fn begin_transaction_compatibility_alias_is_preserved() {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy(&crate::test_support::database_url())
        .expect("construct lazy compatibility pool");
    pool.close().await;
    let db = Db::from_pool(pool);
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _guard = metrics::set_default_local_recorder(&recorder);

    #[allow(deprecated)]
    let result = db.begin_transaction().await;
    assert!(matches!(
        result,
        Err(DbError::Sqlx(sqlx::Error::PoolClosed))
    ));

    let counters = snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .filter_map(|(key, _, _, value)| {
            let name = key.key().name();
            if ![
                "buzz_db_pool_acquire_attempts_total",
                "buzz_db_pool_acquisitions_total",
            ]
            .contains(&name)
            {
                return None;
            }
            let DebugValue::Counter(value) = value else {
                panic!("pool acquisition terminals must be counters");
            };
            let labels = key
                .key()
                .labels()
                .map(|label| (label.key().to_owned(), label.value().to_owned()))
                .collect::<std::collections::BTreeMap<_, _>>();
            Some(((name.to_owned(), labels), value))
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let expected = [
        (
            (
                "buzz_db_pool_acquire_attempts_total".to_owned(),
                [
                    ("operation".to_owned(), "event_write".to_owned()),
                    ("outcome".to_owned(), "error".to_owned()),
                    ("pool_role".to_owned(), "writer".to_owned()),
                ]
                .into_iter()
                .collect(),
            ),
            1,
        ),
        (
            (
                "buzz_db_pool_acquisitions_total".to_owned(),
                [
                    ("outcome".to_owned(), "error".to_owned()),
                    ("pool_role".to_owned(), "writer".to_owned()),
                ]
                .into_iter()
                .collect(),
            ),
            1,
        ),
    ]
    .into_iter()
    .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(counters, expected);
}

#[test]
fn nip43_reconciliation_compatibility_alias_is_preserved() {
    #[allow(deprecated)]
    async fn call(
        db: &Db,
        community_id: CommunityId,
        relay_pubkey: &nostr::PublicKey,
    ) -> crate::Result<bool> {
        db.nip43_membership_snapshot_needs_reconciliation(community_id, relay_pubkey)
            .await
    }

    let _ = call;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn readiness_check_distinguishes_pool_exhaustion_from_success() {
    let database_url = crate::test_support::database_url();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect size-one readiness test pool");
    let held = pool
        .acquire()
        .await
        .expect("hold the only readiness test connection");
    let db = Db::from_pool(pool);

    let exhausted = db
        .readiness_check(tokio::time::Instant::now() + std::time::Duration::from_millis(25))
        .await;
    assert_eq!(exhausted, DbReadinessOutcome::PoolTimeout);

    drop(held);
    let recovered = db
        .readiness_check(tokio::time::Instant::now() + std::time::Duration::from_secs(1))
        .await;
    assert_eq!(recovered, DbReadinessOutcome::Success);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn readiness_check_classifies_closed_pool_query_timeout_and_query_error() {
    let database_url = crate::test_support::database_url();

    let closed_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect closed readiness test pool");
    closed_pool.close().await;
    let closed = Db::from_pool(closed_pool)
        .readiness_check(tokio::time::Instant::now() + std::time::Duration::from_secs(1))
        .await;
    assert_eq!(closed, DbReadinessOutcome::PoolError);

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect query classification test pool");
    let db = Db::from_pool(pool);

    let timed_out = db
        .readiness_check_sql(
            tokio::time::Instant::now() + std::time::Duration::from_millis(25),
            "SELECT pg_sleep(0.2)",
        )
        .await;
    assert_eq!(timed_out, DbReadinessOutcome::QueryTimeout);

    let query_error = db
        .readiness_check_sql(
            tokio::time::Instant::now() + std::time::Duration::from_secs(1),
            "SELECT 1 / 0",
        )
        .await;
    assert_eq!(query_error, DbReadinessOutcome::QueryError);

    assert_eq!(
        db.readiness_check(tokio::time::Instant::now() + std::time::Duration::from_secs(1))
            .await,
        DbReadinessOutcome::Success,
        "query failures must return the acquired connection to the pool"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn readiness_check_cancellation_balances_waiter_and_inflight_connection() {
    let database_url = crate::test_support::database_url();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect cancellation readiness test pool");
    let held = pool
        .acquire()
        .await
        .expect("hold sole connection before waiter cancellation");
    let db = Db::from_pool(pool);

    let waiting_db = db.clone();
    let waiting = tokio::spawn(async move {
        waiting_db
            .readiness_check(tokio::time::Instant::now() + std::time::Duration::from_secs(5))
            .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    waiting.abort();
    assert!(waiting
        .await
        .expect_err("waiting check must be cancelled")
        .is_cancelled());
    drop(held);

    assert_eq!(
        db.readiness_check(tokio::time::Instant::now() + std::time::Duration::from_secs(1))
            .await,
        DbReadinessOutcome::Success,
        "cancelled pool waiter must not consume the released connection"
    );

    let querying_db = db.clone();
    let querying = tokio::spawn(async move {
        querying_db
            .readiness_check_sql(
                tokio::time::Instant::now() + std::time::Duration::from_secs(5),
                "SELECT pg_sleep(5)",
            )
            .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    querying.abort();
    assert!(querying
        .await
        .expect_err("querying check must be cancelled")
        .is_cancelled());

    let recovered = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let outcome = db
                .readiness_check(
                    tokio::time::Instant::now() + std::time::Duration::from_millis(250),
                )
                .await;
            match outcome {
                DbReadinessOutcome::Success => break outcome,
                DbReadinessOutcome::PoolTimeout => tokio::task::yield_now().await,
                unexpected => panic!(
                    "cancelled in-flight query produced unexpected recovery outcome: {unexpected:?}"
                ),
            }
        }
    })
    .await
    .expect("cancelled in-flight query must return or replace its connection");
    assert_eq!(recovered, DbReadinessOutcome::Success);
}

async fn make_community(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    let host = format!("communities-of-channels-{}.example", id.simple());
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(id)
        .bind(host)
        .execute(pool)
        .await
        .expect("insert community");
    id
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_database_guard_covers_legacy_writer_and_nip09_deletion() {
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    let db = setup_db().await;
    let community = CommunityId::from_uuid(make_community(&db.pool).await);
    let keys = Keys::generate();
    let d_tag = format!("read-state:{}", "b".repeat(32));
    let tags = vec![
        Tag::parse(["d", d_tag.as_str()]).expect("d tag"),
        Tag::parse(["t", "read-state"]).expect("t tag"),
    ];
    let base = Timestamp::now().as_secs();
    let a = EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_READ_STATE as u16), "A")
        .tags(tags.clone())
        .custom_created_at(Timestamp::from(base))
        .sign_with_keys(&keys)
        .expect("sign A");
    let x = EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_READ_STATE as u16), "X")
        .tags(tags.clone())
        .custom_created_at(Timestamp::from(base + 1))
        .sign_with_keys(&keys)
        .expect("sign X");
    let b = EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_READ_STATE as u16), "B")
        .tags(tags.clone())
        .custom_created_at(Timestamp::from(base + 2))
        .sign_with_keys(&keys)
        .expect("sign B");
    let c = EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_READ_STATE as u16), "C")
        .tags(tags)
        .custom_created_at(Timestamp::from(base + 3))
        .sign_with_keys(&keys)
        .expect("sign C");

    async fn legacy_insert(
        pool: &PgPool,
        community: CommunityId,
        event: &nostr::Event,
        d_tag: &str,
    ) -> std::result::Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
        sqlx::query(
                "INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig, received_at, d_tag) \
                 VALUES ($1, $2, $3, to_timestamp($4), $5, $6, $7, $8, NOW(), $9) ON CONFLICT DO NOTHING",
            )
            .bind(community.as_uuid())
            .bind(event.id.as_bytes().as_slice())
            .bind(event.pubkey.to_bytes())
            .bind(event.created_at.as_secs() as f64)
            .bind(buzz_core::kind::KIND_READ_STATE as i32)
            .bind(serde_json::to_value(&event.tags).expect("serialize tags"))
            .bind(&event.content)
            .bind(event.sig.serialize().as_slice())
            .bind(d_tag)
            .execute(pool)
            .await
    }

    legacy_insert(&db.pool, community, &a, &d_tag)
        .await
        .expect("legacy insert A");
    let duplicate = legacy_insert(&db.pool, community, &a, &d_tag)
        .await
        .expect("legacy duplicate A remains idempotent");
    assert_eq!(duplicate.rows_affected(), 0);

    sqlx::query(
        "INSERT INTO event_mentions \
                 (community_id, pubkey_hex, event_id, event_created_at, event_kind) \
             VALUES ($1, $2, $3, to_timestamp($4), 30078)",
    )
    .bind(community.as_uuid())
    .bind("c".repeat(64))
    .bind(a.id.as_bytes().as_slice())
    .bind(a.created_at.as_secs() as f64)
    .execute(&db.pool)
    .await
    .expect("insert live mention");

    // Emulate the pre-PR replacement path after migration 0007: soft-delete
    // the live row, then insert B without any application watermark write.
    sqlx::query(
            "UPDATE events SET deleted_at=NOW() \
             WHERE community_id=$1 AND kind=30078 AND pubkey=$2 AND d_tag=$3 AND deleted_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(keys.public_key().to_bytes())
        .bind(&d_tag)
        .execute(&db.pool)
        .await
        .expect("legacy soft-delete A");
    let mentions_after_delete: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM event_mentions WHERE community_id=$1 AND event_id=$2",
    )
    .bind(community.as_uuid())
    .bind(a.id.as_bytes().as_slice())
    .fetch_one(&db.pool)
    .await
    .expect("count mentions after delete");
    assert_eq!(mentions_after_delete, 0);

    let stale_mention = sqlx::query(
        "INSERT INTO event_mentions \
                 (community_id, pubkey_hex, event_id, event_created_at, event_kind) \
             VALUES ($1, $2, $3, to_timestamp($4), 30078)",
    )
    .bind(community.as_uuid())
    .bind("d".repeat(64))
    .bind(a.id.as_bytes().as_slice())
    .bind(a.created_at.as_secs() as f64)
    .execute(&db.pool)
    .await
    .expect("stale post-commit mention is skipped");
    assert_eq!(stale_mention.rows_affected(), 0);

    legacy_insert(&db.pool, community, &b, &d_tag)
        .await
        .expect("legacy insert B");
    let duplicate_b = legacy_insert(&db.pool, community, &b, &d_tag)
        .await
        .expect("live duplicate B is skipped");
    assert_eq!(duplicate_b.rows_affected(), 0);

    sqlx::query(
        "INSERT INTO event_mentions \
                 (community_id, pubkey_hex, event_id, event_created_at, event_kind) \
             VALUES ($1, $2, $3, to_timestamp($4), 30078)",
    )
    .bind(community.as_uuid())
    .bind("e".repeat(64))
    .bind(b.id.as_bytes().as_slice())
    .bind(b.created_at.as_secs() as f64)
    .execute(&db.pool)
    .await
    .expect("insert B mention");

    // Exercise the new Rust hard-delete path independently. An in-flight
    // mention holds KEY SHARE on B, so replacement by C must block, then
    // complete after the mention commits and remove both B and its mention.
    let mut rust_mention_tx = db
        .pool
        .begin()
        .await
        .expect("begin Rust mention transaction");
    sqlx::query(
        "INSERT INTO event_mentions \
                 (community_id, pubkey_hex, event_id, event_created_at, event_kind) \
             VALUES ($1, $2, $3, to_timestamp($4), 30078) ON CONFLICT DO NOTHING",
    )
    .bind(community.as_uuid())
    .bind("e".repeat(64))
    .bind(b.id.as_bytes().as_slice())
    .bind(b.created_at.as_secs() as f64)
    .execute(&mut *rust_mention_tx)
    .await
    .expect("hold B live-event key-share lock");

    let replace_db = db.clone();
    let replace_d_tag = d_tag.clone();
    let replace_c = c.clone();
    let replace_task = tokio::spawn(async move {
        replace_db
            .replace_parameterized_event(community, &replace_c, &replace_d_tag, None)
            .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        !replace_task.is_finished(),
        "Rust hard delete should wait for mention lock"
    );
    rust_mention_tx
        .commit()
        .await
        .expect("release Rust mention lock");
    let replaced = tokio::time::timeout(std::time::Duration::from_secs(2), replace_task)
        .await
        .expect("Rust hard delete deadlocked with mention insert")
        .expect("replacement task panicked")
        .expect("replace B with C");
    assert!(replaced.1, "C must replace B");
    let b_mentions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM event_mentions WHERE community_id=$1 AND event_id=$2",
    )
    .bind(community.as_uuid())
    .bind(b.id.as_bytes().as_slice())
    .fetch_one(&db.pool)
    .await
    .expect("count B mentions after Rust replacement");
    assert_eq!(b_mentions, 0);

    sqlx::query(
        "INSERT INTO event_mentions \
                 (community_id, pubkey_hex, event_id, event_created_at, event_kind) \
             VALUES ($1, $2, $3, to_timestamp($4), 30078)",
    )
    .bind(community.as_uuid())
    .bind("f".repeat(64))
    .bind(c.id.as_bytes().as_slice())
    .bind(c.created_at.as_secs() as f64)
    .execute(&db.pool)
    .await
    .expect("insert C mention");

    // Exercise legacy UPDATE-trigger deletion with the same barrier. While
    // deletion waits on C's KEY SHARE lock, an exact replay must already be
    // a zero-row trigger no-op; it must not wait for deletion or resurrect C.
    let mut legacy_mention_tx = db
        .pool
        .begin()
        .await
        .expect("begin legacy mention transaction");
    sqlx::query(
        "INSERT INTO event_mentions \
                 (community_id, pubkey_hex, event_id, event_created_at, event_kind) \
             VALUES ($1, $2, $3, to_timestamp($4), 30078) ON CONFLICT DO NOTHING",
    )
    .bind(community.as_uuid())
    .bind("f".repeat(64))
    .bind(c.id.as_bytes().as_slice())
    .bind(c.created_at.as_secs() as f64)
    .execute(&mut *legacy_mention_tx)
    .await
    .expect("hold C live-event key-share lock");

    let delete_pool = db.pool.clone();
    let delete_pubkey = keys.public_key().to_bytes();
    let delete_d_tag = d_tag.clone();
    let delete_task = tokio::spawn(async move {
        sqlx::query(
                "UPDATE events SET deleted_at=NOW() \
                 WHERE community_id=$1 AND kind=30078 AND pubkey=$2 AND d_tag=$3 AND deleted_at IS NULL",
            )
            .bind(community.as_uuid())
            .bind(delete_pubkey)
            .bind(delete_d_tag)
            .execute(&delete_pool)
            .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        !delete_task.is_finished(),
        "legacy delete should wait for mention lock"
    );

    let replay_while_delete_waits = legacy_insert(&db.pool, community, &c, &d_tag)
        .await
        .expect("concurrent exact C replay is skipped");
    assert_eq!(replay_while_delete_waits.rows_affected(), 0);

    legacy_mention_tx
        .commit()
        .await
        .expect("release legacy mention lock");
    tokio::time::timeout(std::time::Duration::from_secs(2), delete_task)
        .await
        .expect("legacy delete deadlocked with mention insert")
        .expect("delete task panicked")
        .expect("legacy NIP-09 delete C");

    let payloads: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM events WHERE community_id=$1 AND kind=30078 AND pubkey=$2 AND d_tag=$3",
        )
        .bind(community.as_uuid())
        .bind(keys.public_key().to_bytes())
        .bind(&d_tag)
        .fetch_one(&db.pool)
        .await
        .expect("count retained payloads");
    assert_eq!(
        payloads, 0,
        "legacy soft deletes must not retain NIP-RS payloads"
    );

    // Opposite commit order: deletion has committed before exact replay.
    // Equality remains an observable zero-row no-op, never a resurrection.
    let replay_c = legacy_insert(&db.pool, community, &c, &d_tag)
        .await
        .expect("post-delete exact C replay is skipped");
    assert_eq!(replay_c.rows_affected(), 0);
    let payloads_after_exact_replay: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM events WHERE community_id=$1 AND kind=30078 AND pubkey=$2 AND d_tag=$3",
        )
        .bind(community.as_uuid())
        .bind(keys.public_key().to_bytes())
        .bind(&d_tag)
        .fetch_one(&db.pool)
        .await
        .expect("count payloads after exact replay");
    assert_eq!(payloads_after_exact_replay, 0);

    let replay = legacy_insert(&db.pool, community, &x, &d_tag).await;
    assert!(
        replay.is_err(),
        "database guard must reject A < X < C replay"
    );

    let watermark: (chrono::DateTime<chrono::Utc>, Vec<u8>) = sqlx::query_as(
        "SELECT created_at, event_id FROM parameterized_event_watermarks \
             WHERE community_id=$1 AND kind=30078 AND pubkey=$2 AND d_tag=$3",
    )
    .bind(community.as_uuid())
    .bind(keys.public_key().to_bytes())
    .bind(&d_tag)
    .fetch_one(&db.pool)
    .await
    .expect("read C watermark");
    assert_eq!(watermark.0.timestamp(), base as i64 + 3);
    assert_eq!(watermark.1, c.id.as_bytes().as_slice());
}

// ---- Read-replica routing ------------------------------------------------
//
// These tests pin the routing contract of `Db::read()` and the two routed
// methods. A second scratch database stands in for the replica; the
// fixtures are deliberately DIVERGENT (rows that exist in only one of the
// two databases) so every assertion observes which pool actually served
// the query instead of trusting the routing code's word for it.

async fn admin_url() -> String {
    std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| TEST_DB_URL.into())
}

/// Create a fresh scratch database on the same server and optionally run migrations.
async fn create_scratch_db_through(
    admin: &PgPool,
    prefix: &str,
    target: Option<i64>,
) -> (PgPool, String) {
    let name = format!("{}_{}", prefix, Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .execute(admin)
        .await
        .expect("create scratch db");
    let base = admin_url().await;
    // Swap the database path segment of the admin URL for the scratch name.
    let scratch_url = {
        let idx = base.rfind('/').expect("db url has a path segment");
        format!("{}/{}", &base[..idx], name)
    };
    let pool = PgPool::connect(&scratch_url)
        .await
        .expect("connect scratch db");
    match target {
        Some(target) => migration::run_migrations_through(&pool, target)
            .await
            .expect("migrate scratch db through target"),
        None => migration::run_migrations(&pool)
            .await
            .expect("migrate scratch db"),
    }
    (pool, name)
}

/// Create a fresh scratch database on the same server and run all migrations.
/// Returns (pool, db_name); callers should `drop_scratch_db` when done.
async fn create_scratch_db(admin: &PgPool, prefix: &str) -> (PgPool, String) {
    create_scratch_db_through(admin, prefix, None).await
}

async fn drop_scratch_db(admin: &PgPool, pool: PgPool, name: &str) {
    pool.close().await;
    let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
    )))
    .execute(admin)
    .await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn push_gateway_profile_migration_converges_brownfield_authority() {
    let admin = PgPool::connect(&admin_url().await)
        .await
        .expect("connect admin database");
    let (pool, name) = create_scratch_db_through(&admin, "push_profile", Some(42)).await;
    let installation_id = Uuid::new_v4();
    let now = chrono::Utc::now();

    sqlx::query(
        "INSERT INTO push_gateway_installations(\
         id, app_attest_key_id, app_attest_public_key, assertion_counter, app_profile, \
         token_ciphertext, token_fingerprint, endpoint_epoch, expires_at) \
         VALUES($1, $2, $3, 0, 'buzz-ios-production', $4, $5, 1, $6)",
    )
    .bind(installation_id)
    .bind(vec![1_u8])
    .bind(vec![2_u8; 33])
    .bind(vec![3_u8])
    .bind(vec![4_u8; 32])
    .bind(now + chrono::Duration::days(1))
    .execute(&pool)
    .await
    .expect("insert legacy production installation");
    sqlx::query(
        "INSERT INTO push_gateway_delegations(\
         id, installation_id, relay_pubkey, endpoint_epoch, generation, not_before, expires_at) \
         VALUES($1, $2, $3, 1, 1, $4, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(installation_id)
    .bind(vec![5_u8; 32])
    .bind(now)
    .bind(now + chrono::Duration::hours(1))
    .execute(&pool)
    .await
    .expect("insert delegation for legacy installation");

    migration::run_migrations(&pool)
        .await
        .expect("apply dogfood-only migration");

    let legacy_installations: i64 =
        sqlx::query_scalar("SELECT count(*) FROM push_gateway_installations")
            .fetch_one(&pool)
            .await
            .expect("count legacy installations");
    let legacy_delegations: i64 =
        sqlx::query_scalar("SELECT count(*) FROM push_gateway_delegations")
            .fetch_one(&pool)
            .await
            .expect("count legacy delegations");
    assert_eq!(legacy_installations, 0);
    assert_eq!(legacy_delegations, 0);

    sqlx::query(
        "INSERT INTO push_gateway_installations(\
         id, app_attest_key_id, app_attest_public_key, assertion_counter, app_profile, \
         token_ciphertext, token_fingerprint, endpoint_epoch, expires_at) \
         VALUES($1, $2, $3, 0, 'buzz-ios-dogfood', $4, $5, 1, $6)",
    )
    .bind(Uuid::new_v4())
    .bind(vec![6_u8])
    .bind(vec![7_u8; 33])
    .bind(vec![8_u8])
    .bind(vec![9_u8; 32])
    .bind(now + chrono::Duration::days(1))
    .execute(&pool)
    .await
    .expect("dogfood installation is accepted after migration");

    let sandbox = sqlx::query(
        "INSERT INTO push_gateway_installations(\
         id, app_attest_key_id, app_attest_public_key, assertion_counter, app_profile, \
         token_ciphertext, token_fingerprint, endpoint_epoch, expires_at) \
         VALUES($1, $2, $3, 0, 'buzz-ios-sandbox', $4, $5, 1, $6)",
    )
    .bind(Uuid::new_v4())
    .bind(vec![10_u8])
    .bind(vec![11_u8; 33])
    .bind(vec![12_u8])
    .bind(vec![13_u8; 32])
    .bind(now + chrono::Duration::days(1))
    .execute(&pool)
    .await;
    assert!(sandbox.is_err(), "legacy sandbox profile must be rejected");

    drop_scratch_db(&admin, pool, &name).await;
    admin.close().await;
}

/// Insert identical community + channel rows into a database so the same
/// (community, channel) ids resolve in both writer and replica.
async fn seed_community_channel(
    pool: &PgPool,
    community: Uuid,
    channel: Uuid,
    author: &nostr::Keys,
) {
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(community)
        .bind(format!("replica-routing-{}.example", community.simple()))
        .execute(pool)
        .await
        .expect("insert community");
    crate::channel::create_channel_with_id(
        pool,
        CommunityId::from_uuid(community),
        channel,
        &format!("replica-routing-{channel}"),
        crate::channel::ChannelType::Stream,
        crate::channel::ChannelVisibility::Open,
        None,
        author.public_key().to_bytes().as_slice(),
        None,
    )
    .await
    .expect("create channel");
}

fn signed_event_at(keys: &nostr::Keys, content: &str, secs: u64) -> nostr::Event {
    nostr::EventBuilder::new(nostr::Kind::Custom(9), content)
        .custom_created_at(nostr::Timestamp::from(secs))
        .sign_with_keys(keys)
        .expect("sign event")
}

async fn insert_top_level(pool: &PgPool, community: Uuid, channel: Uuid, ev: &nostr::Event) {
    let ts = chrono::DateTime::from_timestamp(ev.created_at.as_secs() as i64, 0).expect("valid ts");
    event::insert_event_with_thread_metadata(
        pool,
        CommunityId::from_uuid(community),
        ev,
        Some(channel),
        Some(event::ThreadMetadataParams {
            event_id: ev.id.as_bytes(),
            event_created_at: ts,
            channel_id: channel,
            parent_event_id: None,
            parent_event_created_at: None,
            root_event_id: None,
            root_event_created_at: None,
            depth: 0,
            broadcast: true,
        }),
    )
    .await
    .expect("insert top-level event");
}

async fn insert_thread_reply(
    pool: &PgPool,
    community: Uuid,
    channel: Uuid,
    root: &nostr::Event,
    reply: &nostr::Event,
) {
    let reply_ts =
        chrono::DateTime::from_timestamp(reply.created_at.as_secs() as i64, 0).expect("valid ts");
    let root_ts =
        chrono::DateTime::from_timestamp(root.created_at.as_secs() as i64, 0).expect("valid ts");
    event::insert_event_with_thread_metadata(
        pool,
        CommunityId::from_uuid(community),
        reply,
        Some(channel),
        Some(event::ThreadMetadataParams {
            event_id: reply.id.as_bytes(),
            event_created_at: reply_ts,
            channel_id: channel,
            parent_event_id: Some(root.id.as_bytes()),
            parent_event_created_at: Some(root_ts),
            root_event_id: Some(root.id.as_bytes()),
            root_event_created_at: Some(root_ts),
            depth: 1,
            broadcast: false,
        }),
    )
    .await
    .expect("insert reply");
}

/// Composite thread cursor: 8-byte BE seconds + raw event id.
fn thread_cursor(reply: &crate::thread::ThreadReply) -> Vec<u8> {
    let mut cur = reply.created_at.timestamp().to_be_bytes().to_vec();
    cur.extend_from_slice(&reply.event_id);
    cur
}

#[tokio::test]
async fn read_falls_back_to_writer_when_no_replica_configured() {
    // Pure wiring test — connect_lazy never touches the network.
    let pool = sqlx::PgPool::connect_lazy(TEST_DB_URL).expect("lazy pool");
    let db = Db::from_pool(pool);
    assert!(!db.has_read_pool());
    assert!(
        std::ptr::eq(db.read(), &db.pool),
        "read() must be the writer pool when no replica is configured"
    );
    assert!(db.read_pool_stats().is_none());
}

#[test]
fn read_budget_zero_disables_and_large_values_clamp_to_staleness() {
    assert_eq!(read_budget_from_ms(0), None, "0 = bounded routing off");
    assert_eq!(
        read_budget_from_ms(1000),
        Some(std::time::Duration::from_millis(1000))
    );
    assert_eq!(
        read_budget_from_ms(10_000_000),
        Some(replica_fence::FENCE_STALENESS),
        "budgets above the staleness gate clamp to it"
    );
}

/// Truth table for [`RoutePredicate::for_query`]: the strongest sound
/// predicate per query shape, and — the deploy-day default row — that
/// `routing_enabled = false` (BUZZ_REPLICA_READ_MAX_AGE_MS unset)
/// forces `Bounded` even for covered-eligible shapes, so the zero
/// budget fails the new seams closed (Dawn's covered-at-zero-budget
/// catch, design doc rev 5).
#[test]
fn for_query_predicate_truth_table() {
    let community = CommunityId::from_uuid(Uuid::new_v4());
    let channel = Uuid::new_v4();
    let until = chrono::Utc::now();

    let pinned_with_until = {
        let mut q = event::EventQuery::for_community(community);
        q.channel_id = Some(channel);
        q.until = Some(until);
        q
    };
    let pinned_no_until = {
        let mut q = event::EventQuery::for_community(community);
        q.channel_id = Some(channel);
        q
    };
    let unpinned_with_until = {
        let mut q = event::EventQuery::for_community(community);
        q.until = Some(until);
        q
    };
    let global_only = {
        let mut q = event::EventQuery::for_community(community);
        q.global_only = true;
        q.until = Some(until);
        q
    };

    // Deploy-day default: budget unset ⇒ Bounded regardless of shape.
    // The zero budget then fails Bounded closed, so the new seams
    // record writer/disabled — merging with no env var set is a no-op.
    assert!(
        matches!(
            RoutePredicate::for_query(&pinned_with_until, false),
            RoutePredicate::Bounded
        ),
        "budget unset must not reach the covered arm even when eligible"
    );

    // Budget set + channel pin + until ⇒ the strongest predicate.
    assert!(matches!(
        RoutePredicate::for_query(&pinned_with_until, true),
        RoutePredicate::BoundedOrCovered { .. }
    ));

    // Missing either covered precondition ⇒ Bounded.
    assert!(matches!(
        RoutePredicate::for_query(&pinned_no_until, true),
        RoutePredicate::Bounded
    ));
    assert!(matches!(
        RoutePredicate::for_query(&unpinned_with_until, true),
        RoutePredicate::Bounded
    ));
    // global_only implies `channel_id = None`, so the channel-pin
    // precondition fails and no covered arm is possible — `for_query`
    // never inspects `global_only` itself; the row holds because
    // constructor 1 (channel pin) returns None for an unpinned query.
    assert!(matches!(
        RoutePredicate::for_query(&global_only, true),
        RoutePredicate::Bounded
    ));
}

/// The pre-existing cursor paths are NOT budget-gated: a channel-window
/// cursor page still derives `Covered` with no `routing_enabled` input
/// at all — at B=0 today it routes covered, and that status quo is
/// intentionally unchanged by the `for_query` gate (Max's matrix row:
/// old paths route at budget-unset; only the new seams go dark).
#[test]
fn channel_cursor_predicate_is_not_budget_gated() {
    let channel = Uuid::new_v4();
    let cursor = Some((chrono::Utc::now(), vec![1u8; 32]));
    assert!(matches!(
        RoutePredicate::from_channel_cursor(channel, &cursor),
        RoutePredicate::Covered { .. }
    ));
    // Head fetch (no cursor) is bounded — gated by the budget.
    assert!(matches!(
        RoutePredicate::from_channel_cursor(channel, &None),
        RoutePredicate::Bounded
    ));
}

/// D5 wiring: `read_pool_stats().max` must be the READER pool's own
/// ceiling, not the writer's — `buzz_db_read_pool_active / _max` is the
/// operator's utilisation signal and inheriting the writer's max hides
/// reader saturation by exactly the sizing ratio. Pure wiring test:
/// `connect_lazy` never touches the network, but it does spawn the
/// pool reaper task, which needs a Tokio runtime — hence
/// `#[tokio::test]` despite the test body itself never awaiting.
#[tokio::test]
async fn read_pool_stats_reports_reader_ceiling_not_writer() {
    let writer = sqlx::postgres::PgPoolOptions::new()
        .max_connections(20)
        .connect_lazy(TEST_DB_URL)
        .expect("lazy writer pool");
    let reader = sqlx::postgres::PgPoolOptions::new()
        .max_connections(40)
        .connect_lazy(TEST_DB_URL)
        .expect("lazy reader pool");
    let db = Db::from_pools(writer, reader);
    assert_eq!(db.pool_stats().max, 20);
    assert_eq!(
        db.read_pool_stats().expect("read pool configured").max,
        40,
        "reader gauge must report the reader's own ceiling"
    );
}

/// D4 wiring: the reader pool is built lazily with `min_connections(0)`
/// and the short reader acquire timeout — construction must succeed
/// with no replica listening (reader-down at boot must not crash the
/// relay), and `read_max_connections` must honour
/// `DbConfig::read_max_connections` over the writer sizing.
/// `#[tokio::test]` because `connect_lazy` spawns the pool reaper task,
/// which needs a Tokio runtime even though nothing is dialed.
#[tokio::test]
async fn connect_read_pool_is_lazy_and_independently_sized() {
    let config = DbConfig {
        max_connections: 20,
        read_max_connections: Some(7),
        ..DbConfig::default()
    };
    // Unroutable per RFC 5737 TEST-NET-1: proves nothing is dialed at
    // construction time.
    let pool = Db::connect_read_pool(&config, "postgres://user:pw@192.0.2.1:5432/none", 7)
        .expect("lazy construction must not dial the replica");
    assert_eq!(pool.options().get_max_connections(), 7);
    assert_eq!(pool.options().get_min_connections(), 0);
    assert_eq!(
        pool.options().get_acquire_timeout(),
        Db::READER_ACQUIRE_TIMEOUT
    );
}

/// Channel window: head fetch (no cursor) reads the WRITER; cursor pages
/// read the REPLICA. Divergent fixtures prove which pool served each.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn channel_window_routes_head_to_writer_and_cursor_pages_to_replica() {
    let admin = PgPool::connect(&admin_url().await)
        .await
        .expect("connect admin");
    let (writer, wname) = create_scratch_db(&admin, "routing_w").await;
    let (replica, rname) = create_scratch_db(&admin, "routing_r").await;

    let author = nostr::Keys::generate();
    let community = Uuid::new_v4();
    let channel = Uuid::new_v4();
    seed_community_channel(&writer, community, channel, &author).await;
    seed_community_channel(&replica, community, channel, &author).await;

    // Shared history (both databases): m1 < m2 < m3.
    let base = 1_700_000_000u64;
    let m1 = signed_event_at(&author, "m1", base);
    let m2 = signed_event_at(&author, "m2", base + 10);
    let m3 = signed_event_at(&author, "m3", base + 20);
    for pool in [&writer, &replica] {
        for ev in [&m1, &m2, &m3] {
            insert_top_level(pool, community, channel, ev).await;
        }
    }
    // Lag: the newest event exists only on the writer.
    let fresh = signed_event_at(&author, "fresh-writer-only", base + 30);
    insert_top_level(&writer, community, channel, &fresh).await;
    // Marker: exists only on the "replica" (unphysical for a real replica,
    // but it makes replica-served pages unambiguous).
    let marker = signed_event_at(&author, "replica-only-marker", base + 5);
    insert_top_level(&replica, community, channel, &marker).await;

    let db = Db::from_pools(writer.clone(), replica.clone());
    // Open the fence through "now": the fixture's history is far in the
    // past, so every cursor falls below the fence and routing is
    // eligible. Fence-gating itself is pinned by the fence tests below.
    db.fence().force_open_for_tests(chrono::Utc::now());
    let cid = CommunityId::from_uuid(community);

    // Head fetch (cursor: None) → writer: sees `fresh`, never `marker`.
    let head = db
        .get_channel_window(cid, channel, 2, None, None)
        .await
        .expect("head window");
    let head_contents: Vec<String> = head
        .rows
        .iter()
        .map(|r| r.stored_event.event.content.clone())
        .collect();
    assert_eq!(
        head_contents,
        vec!["fresh-writer-only".to_string(), "m3".to_string()],
        "head fetch must be served by the writer"
    );

    // Cursor page → replica: sees `marker`, never `fresh`.
    let cursor = head.next_cursor.expect("has_more implies next_cursor");
    let page2 = db
        .get_channel_window(cid, channel, 10, Some(cursor), None)
        .await
        .expect("cursor window");
    let page2_contents: Vec<String> = page2
        .rows
        .iter()
        .map(|r| r.stored_event.event.content.clone())
        .collect();
    assert_eq!(
        page2_contents,
        vec![
            "m2".to_string(),
            "replica-only-marker".to_string(),
            "m1".to_string()
        ],
        "cursor page must be served by the replica"
    );

    drop_scratch_db(&admin, replica, &rname).await;
    drop_scratch_db(&admin, writer, &wname).await;
}

/// Fail-closed on a mid-request replica failure (Dawn, review of
/// 1b0aa0dfa): a replica-routed page whose query errors *after* the
/// proof (the live shape is a hot-standby recovery conflict — 40001 /
/// 25P02 — cancelling the held snapshot under `max_standby_streaming_delay`)
/// must be re-run on the writer and served, never surfaced as an error
/// the writer could have answered. Degraded capacity, never holes.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn replica_window_failure_falls_back_to_writer() {
    let admin = PgPool::connect(&admin_url().await)
        .await
        .expect("connect admin");
    let (writer, wname) = create_scratch_db(&admin, "fb_w").await;
    let (replica, rname) = create_scratch_db(&admin, "fb_r").await;

    let author = nostr::Keys::generate();
    let community = Uuid::new_v4();
    let channel = Uuid::new_v4();
    seed_community_channel(&writer, community, channel, &author).await;
    seed_community_channel(&replica, community, channel, &author).await;

    let base = 1_700_000_000u64;
    let m1 = signed_event_at(&author, "m1", base);
    let m2 = signed_event_at(&author, "m2", base + 10);
    let m3 = signed_event_at(&author, "m3", base + 20);
    for pool in [&writer, &replica] {
        for ev in [&m1, &m2, &m3] {
            insert_top_level(pool, community, channel, ev).await;
        }
    }
    let marker = signed_event_at(&author, "replica-only-marker", base + 5);
    insert_top_level(&replica, community, channel, &marker).await;

    let db = Db::from_pools(writer.clone(), replica.clone());
    db.fence().force_open_for_tests(chrono::Utc::now());
    let cid = CommunityId::from_uuid(community);

    let head = db
        .get_channel_window(cid, channel, 1, None, None)
        .await
        .expect("head window");
    let cursor = head.next_cursor.expect("has_more implies next_cursor");

    // Guard against a vacuous pass: the cursor page must actually be
    // replica-eligible before we break the replica.
    let healthy = db
        .get_channel_window(cid, channel, 10, Some(cursor.clone()), None)
        .await
        .expect("healthy cursor window");
    assert!(
        healthy
            .rows
            .iter()
            .any(|r| r.stored_event.event.content == "replica-only-marker"),
        "fixture must route the cursor page to the replica while healthy"
    );

    // Break the replica AFTER the proof point: the heartbeat table stays
    // intact (the observation succeeds), the page query then fails.
    sqlx::query("DROP TABLE events CASCADE")
        .execute(&replica)
        .await
        .expect("drop replica events");

    let page = db
        .get_channel_window(cid, channel, 10, Some(cursor), None)
        .await
        .expect("replica failure must fall back to the writer, not error");
    let contents: Vec<&str> = page
        .rows
        .iter()
        .map(|r| r.stored_event.event.content.as_str())
        .collect();
    assert_eq!(
        contents,
        vec!["m2", "m1"],
        "fallback page must be the writer's answer (no replica marker)"
    );

    drop_scratch_db(&admin, replica, &rname).await;
    drop_scratch_db(&admin, writer, &wname).await;
}

/// [`replica_window_failure_falls_back_to_writer`] for the thread-replies
/// path: a replica-routed thread page whose query errors after the proof
/// re-runs on the writer instead of surfacing an error.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn replica_thread_failure_falls_back_to_writer() {
    let admin = PgPool::connect(&admin_url().await)
        .await
        .expect("connect admin");
    let (writer, wname) = create_scratch_db(&admin, "fbt_w").await;
    let (replica, rname) = create_scratch_db(&admin, "fbt_r").await;

    let author = nostr::Keys::generate();
    let community = Uuid::new_v4();
    let channel = Uuid::new_v4();
    seed_community_channel(&writer, community, channel, &author).await;
    seed_community_channel(&replica, community, channel, &author).await;

    let base = 1_700_000_000u64;
    let root = signed_event_at(&author, "root", base);
    for pool in [&writer, &replica] {
        insert_top_level(pool, community, channel, &root).await;
    }
    let replies: Vec<nostr::Event> = (1..=3)
        .map(|i| signed_event_at(&author, &format!("r{i}"), base + 10 * i as u64))
        .collect();
    for pool in [&writer, &replica] {
        for reply in &replies {
            insert_thread_reply(pool, community, channel, &root, reply).await;
        }
    }
    // Replica-only divergent reply between r2 and r3 marks replica serves.
    let ghost = signed_event_at(&author, "replica-only-ghost", base + 25);
    insert_thread_reply(&replica, community, channel, &root, &ghost).await;

    let db = Db::from_pools(writer.clone(), replica.clone());
    db.fence().force_open_for_tests(chrono::Utc::now());
    let cid = CommunityId::from_uuid(community);

    let page1 = db
        .get_thread_replies(cid, root.id.as_bytes(), Some(10), 2, None)
        .await
        .expect("head page");
    let cur = thread_cursor(page1.last().expect("page 1 non-empty"));

    // Healthy: the full page after r2 is the replica's [ghost].
    let healthy = db
        .get_thread_replies(cid, root.id.as_bytes(), Some(10), 1, Some(&cur))
        .await
        .expect("healthy replica page");
    assert_eq!(
        healthy[0].stored_event.event.content, "replica-only-ghost",
        "fixture must route the cursor page to the replica while healthy"
    );

    sqlx::query("DROP TABLE events CASCADE")
        .execute(&replica)
        .await
        .expect("drop replica events");

    let page = db
        .get_thread_replies(cid, root.id.as_bytes(), Some(10), 1, Some(&cur))
        .await
        .expect("replica failure must fall back to the writer, not error");
    assert_eq!(
        page[0].stored_event.event.content, "r3",
        "fallback page must be the writer's answer"
    );

    drop_scratch_db(&admin, replica, &rname).await;
    drop_scratch_db(&admin, writer, &wname).await;
}

/// Mid-request degradation of the held session (Dawn, review of
/// 1b0aa0dfa): when the proved replica transaction dies between the page
/// and an aux follow-up (stand-in: `pg_terminate_backend` on the reader
/// connection, the same tx-fatal shape as a recovery-conflict cancel),
/// [`ReadSession::query_events`] must re-run the query on the writer and
/// permanently degrade the session instead of surfacing the error.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn read_session_degrades_to_writer_when_replica_connection_dies() {
    let admin = PgPool::connect(&admin_url().await)
        .await
        .expect("connect admin");
    let (writer, wname) = create_scratch_db(&admin, "deg_w").await;
    let (replica, rname) = create_scratch_db(&admin, "deg_r").await;

    let author = nostr::Keys::generate();
    let community = Uuid::new_v4();
    let channel = Uuid::new_v4();
    seed_community_channel(&writer, community, channel, &author).await;
    seed_community_channel(&replica, community, channel, &author).await;

    let base = 1_700_000_000u64;
    let m1 = signed_event_at(&author, "m1", base);
    let m2 = signed_event_at(&author, "m2", base + 10);
    for pool in [&writer, &replica] {
        for ev in [&m1, &m2] {
            insert_top_level(pool, community, channel, ev).await;
        }
    }
    // Writer-only row proves the degraded aux ran on the writer.
    let fresh = signed_event_at(&author, "fresh-writer-only", base + 20);
    insert_top_level(&writer, community, channel, &fresh).await;

    let db = Db::from_pools(writer.clone(), replica.clone());
    db.fence().force_open_for_tests(chrono::Utc::now());
    let cid = CommunityId::from_uuid(community);

    let head = db
        .get_channel_window(cid, channel, 1, None, None)
        .await
        .expect("head window");
    let cursor = head.next_cursor.expect("has_more implies next_cursor");
    let (_window, mut session) = db
        .get_channel_window_with_session(cid, channel, 10, Some(cursor), None)
        .await
        .expect("routed cursor window");
    assert!(
        session.is_replica(),
        "fixture must route this page to the replica"
    );

    // Kill the reader's backend out from under the held transaction.
    sqlx::query(
        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
             WHERE datname = $1 AND pid <> pg_backend_pid()",
    )
    .bind(&rname)
    .execute(&admin)
    .await
    .expect("terminate replica backends");

    let mut aux = EventQuery::for_community(cid);
    aux.channel_id = Some(channel);
    let rows = session
        .query_events(&aux)
        .await
        .expect("session must degrade to the writer, not error");
    assert!(
        rows.iter()
            .any(|se| se.event.content == "fresh-writer-only"),
        "degraded aux must be served by the writer"
    );
    assert!(
        !session.is_replica(),
        "the session must be permanently degraded to the writer"
    );

    drop(session);
    drop_scratch_db(&admin, replica, &rname).await;
    drop_scratch_db(&admin, writer, &wname).await;
}

/// Snapshot continuity (Wren, review of 17ea2ff6a): the routed request
/// runs inside ONE `REPEATABLE READ, READ ONLY` transaction whose first
/// statement was the heartbeat observation — so a row committed on the
/// replica *after* the proof must be invisible to every follow-up
/// statement in the same request (page, participants, aux). This
/// distinguishes the transaction contract from mere connection reuse:
/// autocommit statements on the same backend advance their snapshot
/// per statement and WOULD see the mid-request row.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn routed_request_holds_one_snapshot_across_page_and_aux() {
    let admin = PgPool::connect(&admin_url().await)
        .await
        .expect("connect admin");
    let (writer, wname) = create_scratch_db(&admin, "snap_w").await;
    let (replica, rname) = create_scratch_db(&admin, "snap_r").await;

    let author = nostr::Keys::generate();
    let community = Uuid::new_v4();
    let channel = Uuid::new_v4();
    seed_community_channel(&writer, community, channel, &author).await;
    seed_community_channel(&replica, community, channel, &author).await;

    let base = 1_700_000_000u64;
    let m1 = signed_event_at(&author, "m1", base);
    let m2 = signed_event_at(&author, "m2", base + 10);
    for pool in [&writer, &replica] {
        for ev in [&m1, &m2] {
            insert_top_level(pool, community, channel, ev).await;
        }
    }

    let db = Db::from_pools(writer.clone(), replica.clone());
    db.fence().force_open_for_tests(chrono::Utc::now());
    let cid = CommunityId::from_uuid(community);

    // Head page on the writer yields the cursor for a replica-routed page.
    let head = db
        .get_channel_window(cid, channel, 1, None, None)
        .await
        .expect("head window");
    let cursor = head.next_cursor.expect("has_more implies next_cursor");

    // Route the cursor page to the replica and HOLD the session.
    let (window, mut session) = db
        .get_channel_window_with_session(cid, channel, 10, Some(cursor), None)
        .await
        .expect("routed cursor window");
    assert!(
        session.is_replica(),
        "fixture must route this page to the replica"
    );
    assert_eq!(window.rows.len(), 1, "page after m2 is [m1]");

    // Mid-request: a new event commits on the replica (stands in for
    // replay advancing between the page and the aux closure).
    let mid = signed_event_at(&author, "mid-request-commit", base + 5);
    insert_top_level(&replica, community, channel, &mid).await;

    // A fresh autocommit statement on ANOTHER session sees it — the row
    // is really there (control for the assertion below).
    let mut control = EventQuery::for_community(cid);
    control.channel_id = Some(channel);
    let visible_elsewhere = event::query_events(&replica, &control)
        .await
        .expect("control query");
    assert!(
        visible_elsewhere
            .iter()
            .any(|se| se.event.content == "mid-request-commit"),
        "control: the mid-request row must be committed and visible to a new snapshot"
    );

    // The held request session must NOT see it: its snapshot was
    // anchored by the heartbeat observation, before the commit.
    let mut aux = EventQuery::for_community(cid);
    aux.channel_id = Some(channel);
    let in_request = session.query_events(&aux).await.expect("aux query");
    assert!(
        !in_request
            .iter()
            .any(|se| se.event.content == "mid-request-commit"),
        "request transaction must hold the proof-time snapshot; a \
             mid-request commit leaking in means the aux ran outside the \
             request transaction (autocommit connection reuse)"
    );
    // Rows from the proof-time snapshot are still served.
    assert!(
        in_request.iter().any(|se| se.event.content == "m1"),
        "proof-time rows must remain visible in the request snapshot"
    );

    drop(session);
    drop_scratch_db(&admin, replica, &rname).await;
    drop_scratch_db(&admin, writer, &wname).await;
}

/// Head gate (Predicate A): with the budget unset, a head fetch reads
/// the writer even over an open fence; with a budget set and a fresh
/// proved entry, the head page is served by the replica session
/// (bounded staleness accepted); with a budget the fence entry exceeds,
/// the head page falls back to the writer.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn head_fetch_routes_by_configured_budget() {
    let admin = PgPool::connect(&admin_url().await)
        .await
        .expect("connect admin");
    let (writer, wname) = create_scratch_db(&admin, "head_w").await;
    let (replica, rname) = create_scratch_db(&admin, "head_r").await;

    let author = nostr::Keys::generate();
    let community = Uuid::new_v4();
    let channel = Uuid::new_v4();
    seed_community_channel(&writer, community, channel, &author).await;
    seed_community_channel(&replica, community, channel, &author).await;

    let base = 1_700_000_000u64;
    let shared = signed_event_at(&author, "shared", base);
    for pool in [&writer, &replica] {
        insert_top_level(pool, community, channel, &shared).await;
    }
    // Divergent heads prove which pool served the fetch.
    let fresh = signed_event_at(&author, "fresh-writer-only", base + 30);
    insert_top_level(&writer, community, channel, &fresh).await;
    let marker = signed_event_at(&author, "replica-only-marker", base + 20);
    insert_top_level(&replica, community, channel, &marker).await;

    let mut db = Db::from_pools(writer.clone(), replica.clone());
    db.fence().force_open_for_tests(chrono::Utc::now());
    let cid = CommunityId::from_uuid(community);
    let head_contents = |w: &thread::ChannelWindow| -> Vec<String> {
        w.rows
            .iter()
            .map(|r| r.stored_event.event.content.clone())
            .collect()
    };

    // Budget unset (rollout default): head → writer, fence open or not.
    let head = db
        .get_channel_window(cid, channel, 2, None, None)
        .await
        .expect("head, gate off");
    assert_eq!(
        head_contents(&head),
        vec!["fresh-writer-only".to_string(), "shared".to_string()],
        "head routing must default off"
    );

    // Budget set, entry fresh (just recorded): head → replica.
    db.set_replica_read_max_age_for_tests(Some(std::time::Duration::from_secs(5)));
    let head = db
        .get_channel_window(cid, channel, 2, None, None)
        .await
        .expect("head, gate on");
    assert_eq!(
        head_contents(&head),
        vec!["replica-only-marker".to_string(), "shared".to_string()],
        "a fresh proved entry within budget must serve the head from the replica"
    );

    // Entry older than the budget: head falls back to the writer.
    db.fence().close();
    db.fence().force_open_for_tests_at(
        chrono::Utc::now(),
        std::time::Instant::now() - std::time::Duration::from_secs(10),
    );
    let head = db
        .get_channel_window(cid, channel, 2, None, None)
        .await
        .expect("head, entry too old");
    assert_eq!(
        head_contents(&head),
        vec!["fresh-writer-only".to_string(), "shared".to_string()],
        "an over-budget entry must fail the head gate closed"
    );

    drop_scratch_db(&admin, replica, &rname).await;
    drop_scratch_db(&admin, writer, &wname).await;
}

/// End-to-end deploy-default proof for the NEW routed seams: with the
/// budget unset, a covered-eligible query (channel-pinned + `until`)
/// through [`Db::query_events_routed`] is served by the WRITER — the
/// `for_query` gate keeps the covered arm dark (rev 5). With the budget
/// set and a fresh proved entry, the same query routes to the replica.
/// Divergent fixtures prove which pool served each read.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn query_events_routed_defaults_dark_and_routes_covered_when_enabled() {
    let admin = PgPool::connect(&admin_url().await)
        .await
        .expect("connect admin");
    let (writer, wname) = create_scratch_db(&admin, "qer_w").await;
    let (replica, rname) = create_scratch_db(&admin, "qer_r").await;

    let author = nostr::Keys::generate();
    let community = Uuid::new_v4();
    let channel = Uuid::new_v4();
    seed_community_channel(&writer, community, channel, &author).await;
    seed_community_channel(&replica, community, channel, &author).await;

    let base = 1_700_000_000u64;
    let shared = signed_event_at(&author, "shared", base);
    for pool in [&writer, &replica] {
        insert_top_level(pool, community, channel, &shared).await;
    }
    let writer_only = signed_event_at(&author, "writer-only", base + 10);
    insert_top_level(&writer, community, channel, &writer_only).await;
    let replica_only = signed_event_at(&author, "replica-only", base + 20);
    insert_top_level(&replica, community, channel, &replica_only).await;

    let mut db = Db::from_pools(writer.clone(), replica.clone());
    db.fence().force_open_for_tests(chrono::Utc::now());
    let cid = CommunityId::from_uuid(community);

    // Covered-eligible shape: channel-pinned with an `until` upper
    // bound below the (now) fence wall.
    let q = {
        let mut q = EventQuery::for_community(cid);
        q.channel_id = Some(channel);
        q.until = chrono::DateTime::from_timestamp((base + 60) as i64, 0);
        q
    };
    let contents = |evs: &[StoredEvent]| -> std::collections::BTreeSet<String> {
        evs.iter().map(|e| e.event.content.clone()).collect()
    };

    // Deploy default: budget unset ⇒ writer, even though the shape is
    // covered-eligible and the fence is open.
    let rows = db
        .query_events_routed("test_routed", &q)
        .await
        .expect("routed query, gate off");
    assert!(
        contents(&rows).contains("writer-only"),
        "budget unset must serve the writer"
    );
    assert!(
        !contents(&rows).contains("replica-only"),
        "budget unset must not reach the replica via the covered arm"
    );

    // Budget set ⇒ the covered arm serves it from the replica.
    db.set_replica_read_max_age_for_tests(Some(std::time::Duration::from_secs(5)));
    let rows = db
        .query_events_routed("test_routed", &q)
        .await
        .expect("routed query, gate on");
    assert!(
        contents(&rows).contains("replica-only"),
        "budget set + covered-eligible must route to the replica"
    );
    assert!(!contents(&rows).contains("writer-only"));

    drop_scratch_db(&admin, replica, &rname).await;
    drop_scratch_db(&admin, writer, &wname).await;
}

/// COUNT is bounded-only (rev 5 deletion-visibility rule): a
/// covered-eligible shape must NOT let a count take the covered arm.
/// With the budget unset the count reads the WRITER even with an open
/// fence; with the budget set and a fresh entry it reads the replica
/// under the bounded arm. Divergent row counts prove the serving pool.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn count_events_routed_is_bounded_only() {
    let admin = PgPool::connect(&admin_url().await)
        .await
        .expect("connect admin");
    let (writer, wname) = create_scratch_db(&admin, "cnt_w").await;
    let (replica, rname) = create_scratch_db(&admin, "cnt_r").await;

    let author = nostr::Keys::generate();
    let community = Uuid::new_v4();
    let channel = Uuid::new_v4();
    seed_community_channel(&writer, community, channel, &author).await;
    seed_community_channel(&replica, community, channel, &author).await;

    let base = 1_700_000_000u64;
    // Writer: 2 rows. Replica: 1 row.
    for (i, content) in ["a", "b"].iter().enumerate() {
        let ev = signed_event_at(&author, content, base + i as u64);
        insert_top_level(&writer, community, channel, &ev).await;
    }
    let ev = signed_event_at(&author, "c", base);
    insert_top_level(&replica, community, channel, &ev).await;

    let mut db = Db::from_pools(writer.clone(), replica.clone());
    db.fence().force_open_for_tests(chrono::Utc::now());
    let cid = CommunityId::from_uuid(community);

    // Covered-eligible shape on purpose: pinned + until. A count must
    // ignore that eligibility.
    let q = {
        let mut q = EventQuery::for_community(cid);
        q.channel_id = Some(channel);
        q.until = chrono::DateTime::from_timestamp((base + 60) as i64, 0);
        q
    };

    // Budget unset ⇒ bounded arm disabled ⇒ writer.
    let n = db
        .count_events_routed("test_count", &q)
        .await
        .expect("count, gate off");
    assert_eq!(n, 2, "budget unset must count on the writer");

    // Budget set + fresh entry ⇒ bounded arm ⇒ replica.
    db.set_replica_read_max_age_for_tests(Some(std::time::Duration::from_secs(5)));
    let n = db
        .count_events_routed("test_count", &q)
        .await
        .expect("count, gate on");
    assert_eq!(n, 1, "budget set must count on the replica (bounded)");

    // Entry older than the budget ⇒ bounded fails ⇒ writer. Covered
    // would still hold here (upper <= wall) — proving count never
    // consults it.
    db.fence().close();
    db.fence().force_open_for_tests_at(
        chrono::Utc::now(),
        std::time::Instant::now() - std::time::Duration::from_secs(10),
    );
    let n = db
        .count_events_routed("test_count", &q)
        .await
        .expect("count, entry too old");
    assert_eq!(
        n, 2,
        "an over-budget entry must fail the count closed to the writer, \
             even when the covered arm would admit the shape"
    );

    drop_scratch_db(&admin, replica, &rname).await;
    drop_scratch_db(&admin, writer, &wname).await;
}

/// Routed relay-membership check: budget unset ⇒ writer; budget set +
/// fresh proved entry ⇒ replica (bounded arm); over-budget entry ⇒
/// writer. Divergent membership rows prove which pool answered.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn is_relay_member_is_bounded_routed_and_fails_closed() {
    let admin = PgPool::connect(&admin_url().await)
        .await
        .expect("connect admin");
    let (writer, wname) = create_scratch_db(&admin, "mem_w").await;
    let (replica, rname) = create_scratch_db(&admin, "mem_r").await;

    let community = Uuid::new_v4();
    for pool in [&writer, &replica] {
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community)
            .bind(format!("member-routing-{}.example", community.simple()))
            .execute(pool)
            .await
            .expect("insert community");
    }
    let cid = CommunityId::from_uuid(community);
    let writer_only = "aa".repeat(32);
    let replica_only = "bb".repeat(32);
    relay_members::add_relay_member(&writer, cid, &writer_only, "member", None)
        .await
        .expect("seed writer member");
    relay_members::add_relay_member(&replica, cid, &replica_only, "member", None)
        .await
        .expect("seed replica member");

    let mut db = Db::from_pools(writer.clone(), replica.clone());
    db.fence().force_open_for_tests(chrono::Utc::now());

    // Budget unset ⇒ bounded arm disabled ⇒ writer.
    assert!(
        db.is_relay_member(cid, &writer_only)
            .await
            .expect("gate off"),
        "budget unset must answer from the writer"
    );
    assert!(!db.is_relay_member(cid, &replica_only).await.unwrap());

    // Budget set + fresh entry ⇒ replica.
    db.set_replica_read_max_age_for_tests(Some(std::time::Duration::from_secs(5)));
    assert!(
        db.is_relay_member(cid, &replica_only)
            .await
            .expect("gate on"),
        "budget set must answer from the replica"
    );
    assert!(!db.is_relay_member(cid, &writer_only).await.unwrap());

    // Entry older than the budget ⇒ fail closed to the writer. Close
    // first so no prior fresh entry can be the one proved (matches the
    // count test; today `force_open_for_tests_at` also clears the ring).
    db.fence().close();
    db.fence().force_open_for_tests_at(
        chrono::Utc::now(),
        std::time::Instant::now() - std::time::Duration::from_secs(10),
    );
    assert!(
        db.is_relay_member(cid, &writer_only)
            .await
            .expect("entry too old"),
        "an over-budget entry must fail closed to the writer"
    );

    drop_scratch_db(&admin, replica, &rname).await;
    drop_scratch_db(&admin, writer, &wname).await;
}

/// Community separation across every routed seam, verified on
/// REPLICA-SERVED reads.
///
/// The pre-existing feed/event scoping tests prove the shared SQL
/// builders confine rows to one community, but they exercise those
/// builders through the WRITER wrapper. `_on` variants are
/// executor-only refactors, so scoping *should* be identical — this
/// test refuses to take that on faith and re-proves it through the
/// routed executor, on a snapshot the replica actually served.
///
/// Construction: two communities A and B exist in BOTH databases with
/// the same ids. The replica additionally holds a `replica-only` row in
/// each — divergent fixtures, so any row bearing that content proves
/// the replica (not the writer) served the read. Every assertion
/// requests A and demands B's rows never appear, including B's
/// `replica-only` row, which is the one a leaky predicate would surface.
/// The routed fallback must cost ONE reader acquire budget, even when the
/// Aurora capability cache is cold.
///
/// Regression test for a stacked-budget bug found at `9fa3c9c0b`: the
/// capability probe used to `acquire()` from the pool itself and return
/// `false` *uncached* on `PoolTimedOut`, so the routed read then spent a
/// SECOND `READER_ACQUIRE_TIMEOUT` inside `begin`. Measured 302ms against
/// a ~150ms documented bound. Boot priming
/// ([`Db::spawn_read_pool_boot_ping`]) hid it only when the boot ping
/// SUCCEEDED — and a reader that is unavailable at boot is exactly the
/// case the bound is specified for, so the two failures are correlated.
///
/// The fixture reproduces that state deliberately: a size-1 reader whose
/// sole connection is established and then HELD (so every further acquire
/// must time out), with `reader_aurora_identity` asserted cold. It routes
/// through `count_events_routed` rather than calling `proved_reader`
/// directly, because `buzz_db_route_decision` is emitted by `route_read`
/// — a direct call would prove the timing but never emit the label.
///
/// Timing uses an upper bound of 2x the budget minus a margin: it must
/// fail for two stacked budgets (~300ms) while tolerating scheduler
/// jitter on one (~150ms). Asserting a lower bound too would pin the
/// budget's own value, which `reader_acquire_timeout_is_the_documented_budget`
/// already covers.
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires Postgres"]
async fn routed_fallback_spends_one_acquire_budget_when_aurora_cache_is_cold() {
    let admin = PgPool::connect(&admin_url().await)
        .await
        .expect("connect admin");
    let (seed, wname) = create_scratch_db(&admin, "one_budget").await;
    seed.close().await;
    let base = admin_url().await;
    let scratch_url = {
        let idx = base.rfind('/').expect("db url has a path segment");
        format!("{}/{}", &base[..idx], wname)
    };

    // `Db::new` so the writer arms the floor guard and the reader is the
    // real lazy `connect_read_pool` pool (min_connections=0, 150ms
    // acquire timeout). Reader is sized 1 so holding one connection
    // saturates it.
    let mut db = Db::new(&DbConfig {
        database_url: scratch_url.clone(),
        read_database_url: Some(scratch_url),
        max_connections: 4,
        read_max_connections: Some(1),
        ..DbConfig::default()
    })
    .await
    .expect("connect armed Db with size-1 lazy reader");
    db.fence().force_open_for_tests(chrono::Utc::now());
    db.set_replica_read_max_age_for_tests(Some(Duration::from_secs(5)));

    let read_pool = db.read_pool.clone().expect("reader pool configured");
    // Establish and hold the reader's only connection: saturated.
    let held = read_pool
        .acquire()
        .await
        .expect("establish the reader's sole connection");
    assert_eq!(
        db.read_max_connections, 1,
        "reader max must report 1 for this fixture to test saturation"
    );
    assert_eq!(
        read_pool.size(),
        1,
        "the sole reader connection is established and held"
    );
    // The bug is only observable with the capability cache cold; if a
    // future change primes it here, this fixture would silently stop
    // discriminating.
    assert!(
        db.reader_aurora_identity.get().is_none(),
        "Aurora capability must be UNPRIMED (post-boot-ping-failure state)"
    );

    let recorder = metrics_util::debugging::DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let query = EventQuery::for_community(CommunityId::from_uuid(Uuid::new_v4()));

    // The recorder is installed thread-locally, so it must stay installed
    // across the `.await` — hence the guard form rather than
    // `with_local_recorder`, whose closure cannot host an await. The
    // `current_thread` flavor keeps the route decision on this thread; on
    // a multi-thread runtime the emit could land on a worker where no
    // local recorder is installed and the label assertions would vacuously
    // see an empty snapshot.
    let start = std::time::Instant::now();
    let count = {
        let _guard = metrics::set_default_local_recorder(&recorder);
        db.count_events_routed("one_budget_probe", &query).await
    }
    .expect("writer fallback still answers the read");
    let elapsed = start.elapsed();

    assert_eq!(count, 0, "writer answered on an empty scratch database");
    assert!(
        elapsed < Duration::from_millis(250),
        "routed fallback must spend ONE {}ms acquire budget, not two; took {}ms",
        Db::READER_ACQUIRE_TIMEOUT.as_millis(),
        elapsed.as_millis()
    );

    let reasons: std::collections::HashMap<(String, String), u64> = snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .filter(|(key, ..)| key.key().name() == "buzz_db_route_decision")
        .map(|(key, _, _, value)| {
            let metrics_util::debugging::DebugValue::Counter(n) = value else {
                panic!("buzz_db_route_decision must be a counter");
            };
            let labels: Vec<_> = key.key().labels().collect();
            let get = |name: &str| {
                labels
                    .iter()
                    .find(|l| l.key() == name)
                    .map(|l| l.value().to_owned())
                    .unwrap_or_default()
            };
            ((get("decision"), get("reason")), n)
        })
        .collect();

    assert_eq!(
        reasons.get(&("writer".to_owned(), "reader_acquire_timeout".to_owned())),
        Some(&1),
        "saturated reader must fall back as writer/reader_acquire_timeout; got {reasons:?}"
    );
    // `reader_validation_error` would mean we misclassified a timeout as a
    // broken reader, and `pool_busy` is the retired name — neither may
    // appear in ANY emitted label.
    assert!(
        !reasons
            .keys()
            .any(|(_, reason)| reason == "reader_validation_error" || reason == "pool_busy"),
        "no reader_validation_error or retired pool_busy label may be emitted; got {reasons:?}"
    );

    drop(held);
    drop_scratch_db(&admin, db.pool.clone(), &wname).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn routed_reads_are_confined_to_the_requested_community() {
    let admin = PgPool::connect(&admin_url().await)
        .await
        .expect("connect admin");
    let (writer, wname) = create_scratch_db(&admin, "sep_w").await;
    let (replica, rname) = create_scratch_db(&admin, "sep_r").await;

    let author = nostr::Keys::generate();
    let (comm_a, chan_a) = (Uuid::new_v4(), Uuid::new_v4());
    let (comm_b, chan_b) = (Uuid::new_v4(), Uuid::new_v4());
    for pool in [&writer, &replica] {
        seed_community_channel(pool, comm_a, chan_a, &author).await;
        seed_community_channel(pool, comm_b, chan_b, &author).await;
    }

    // A p-tag mention is what makes a row eligible for the mentions and
    // needs-action feeds. Kind 9 satisfies mentions + activity;
    // needs-action admits only approval/reminder kinds, so each
    // community also gets a kind-46010 row.
    let mentioned = nostr::Keys::generate();
    let mentioned_hex = mentioned.public_key().to_hex();
    let mentioned_bytes = mentioned.public_key().to_bytes();
    let tagged_kind = |kind: u16, content: &str, secs: u64| {
        nostr::EventBuilder::new(nostr::Kind::Custom(kind), content)
            .tags([nostr::Tag::parse(["p", mentioned_hex.as_str()]).expect("p tag")])
            .custom_created_at(nostr::Timestamp::from(secs))
            .sign_with_keys(&author)
            .expect("sign event")
    };
    let tagged = |content: &str, secs: u64| tagged_kind(9, content, secs);

    let base = 1_700_000_000u64;
    // Shared rows (both DBs) + replica-only rows (divergence) per community.
    let a_shared = tagged("a-shared", base);
    let b_shared = tagged("b-shared", base + 1);
    for pool in [&writer, &replica] {
        insert_top_level(pool, comm_a, chan_a, &a_shared).await;
        insert_mentions(
            pool,
            CommunityId::from_uuid(comm_a),
            &a_shared,
            Some(chan_a),
        )
        .await
        .expect("mentions a-shared");
        insert_top_level(pool, comm_b, chan_b, &b_shared).await;
        insert_mentions(
            pool,
            CommunityId::from_uuid(comm_b),
            &b_shared,
            Some(chan_b),
        )
        .await
        .expect("mentions b-shared");
    }
    let a_replica_only = tagged("a-replica-only", base + 10);
    let b_replica_only = tagged("b-replica-only", base + 11);
    insert_top_level(&replica, comm_a, chan_a, &a_replica_only).await;
    insert_mentions(
        &replica,
        CommunityId::from_uuid(comm_a),
        &a_replica_only,
        Some(chan_a),
    )
    .await
    .expect("mentions a-replica-only");
    insert_top_level(&replica, comm_b, chan_b, &b_replica_only).await;
    insert_mentions(
        &replica,
        CommunityId::from_uuid(comm_b),
        &b_replica_only,
        Some(chan_b),
    )
    .await
    .expect("mentions b-replica-only");

    // Needs-action fixtures: approval kind, replica-only in BOTH
    // communities, so the assertion below is replica-served on A and
    // must still not see B's.
    let a_approval = tagged_kind(46010, "a-approval-replica-only", base + 20);
    let b_approval = tagged_kind(46010, "b-approval-replica-only", base + 21);
    insert_top_level(&replica, comm_a, chan_a, &a_approval).await;
    insert_mentions(
        &replica,
        CommunityId::from_uuid(comm_a),
        &a_approval,
        Some(chan_a),
    )
    .await
    .expect("mentions a-approval");
    insert_top_level(&replica, comm_b, chan_b, &b_approval).await;
    insert_mentions(
        &replica,
        CommunityId::from_uuid(comm_b),
        &b_approval,
        Some(chan_b),
    )
    .await
    .expect("mentions b-approval");

    let mut db = Db::from_pools(writer.clone(), replica.clone());
    db.fence().force_open_for_tests(chrono::Utc::now());
    db.set_replica_read_max_age_for_tests(Some(std::time::Duration::from_secs(5)));
    let cid_a = CommunityId::from_uuid(comm_a);

    let contents = |evs: &[StoredEvent]| -> std::collections::BTreeSet<String> {
        evs.iter().map(|e| e.event.content.clone()).collect()
    };
    // Every routed seam must (a) have been served by the replica —
    // proven by a divergent row absent from the writer — and (b) contain
    // no row belonging to community B. All B fixtures are named `b-*`,
    // so the leak check is a single prefix scan.
    let assert_a_only = |rows: &[StoredEvent], marker: &str, seam: &str| {
        let got = contents(rows);
        assert!(
                got.contains(marker),
                "{seam}: must be replica-served (divergent row `{marker}` absent from writer); got {got:?}"
            );
        assert!(
            !got.iter().any(|c| c.starts_with("b-")),
            "{seam}: community B rows leaked into a community A read; got {got:?}"
        );
    };

    // 1. Generic query — covered arm (channel-pinned + `until`).
    let mut q = EventQuery::for_community(cid_a);
    q.channel_id = Some(chan_a);
    q.until = chrono::DateTime::from_timestamp((base + 60) as i64, 0);
    let rows = db
        .query_events_routed("sep_query", &q)
        .await
        .expect("routed query");
    assert_a_only(&rows, "a-replica-only", "query_events_routed");

    // 2. Generic query — bounded arm (no channel pin at all, so a
    //    missing community predicate could not be masked by the pin).
    let unpinned = EventQuery::for_community(cid_a);
    let rows = db
        .query_events_routed_bounded("sep_query_bounded", &unpinned)
        .await
        .expect("routed bounded query");
    assert_a_only(&rows, "a-replica-only", "query_events_routed_bounded");

    // 3. COUNT — bounded-only. Community A holds 3 rows on the replica
    //    (shared + replica-only + approval) but only 1 on the writer,
    //    and 3 more exist in community B. Exactly 3 proves the read was
    //    both replica-served and community-confined.
    let count = db
        .count_events_routed("sep_count", &unpinned)
        .await
        .expect("routed count");
    assert_eq!(
        count, 3,
        "count must see A's three replica rows only — not B's, not the writer's one"
    );

    // 4. By-ID hydration — ids carry no channel pin, and B's ids are
    //    requested alongside A's. Only A's may hydrate.
    let ids: Vec<&[u8]> = vec![
        a_shared.id.as_bytes(),
        a_replica_only.id.as_bytes(),
        b_shared.id.as_bytes(),
        b_replica_only.id.as_bytes(),
    ];
    let rows = db
        .get_events_by_ids_routed("sep_by_ids", cid_a, &ids)
        .await
        .expect("routed by-ids");
    assert_a_only(&rows, "a-replica-only", "get_events_by_ids_routed");

    // 5-7. All three feed builders, each given BOTH channels as
    //      accessible — so only the community predicate can exclude B.
    let both = [chan_a, chan_b];
    let rows = db
        .query_feed_mentions_routed("sep_feed", cid_a, &mentioned_bytes, &both, None, 50)
        .await
        .expect("routed mentions");
    assert_a_only(&rows, "a-replica-only", "query_feed_mentions_routed");

    let rows = db
        .query_feed_needs_action_routed("sep_feed", cid_a, &mentioned_bytes, &both, None, 50)
        .await
        .expect("routed needs action");
    assert_a_only(
        &rows,
        "a-approval-replica-only",
        "query_feed_needs_action_routed",
    );

    let rows = db
        .query_feed_activity_routed("sep_feed", cid_a, &both, None, 50)
        .await
        .expect("routed activity");
    assert_a_only(&rows, "a-replica-only", "query_feed_activity_routed");

    drop_scratch_db(&admin, replica, &rname).await;
    drop_scratch_db(&admin, writer, &wname).await;
}

/// D4: a LAZY reader pool (connect_lazy, min_connections=0, never yet
/// used) must still let [`Db::spawn_fence_probe`] verify the writer's
/// floor guard and spawn — reader-down or reader-idle at boot must not
/// disable fence probing.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn lazy_reader_pool_still_spawns_fence_probe() {
    let admin = PgPool::connect(&admin_url().await)
        .await
        .expect("connect admin");
    let (seed, wname) = create_scratch_db(&admin, "lazy_w").await;
    seed.close().await;

    let writer_url = {
        let base = admin_url().await;
        let idx = base.rfind('/').expect("db url has a path segment");
        format!("{}/{}", &base[..idx], wname)
    };
    // `Db::new` (not `from_pools`) so the WRITER pool arms the
    // `buzz.created_at_floor` GUC — `spawn_fence_probe` verifies the
    // floor guard on a writer connection, and `create_scratch_db`'s
    // plain `PgPool::connect` never arms it. The reader is still the
    // lazy `connect_read_pool` pool this test is about.
    let db = Db::new(&DbConfig {
        database_url: writer_url.clone(),
        read_database_url: Some(writer_url),
        max_connections: 2,
        ..DbConfig::default()
    })
    .await
    .expect("connect armed Db with lazy reader");

    let spawned = db
        .spawn_fence_probe()
        .await
        .expect("floor-guard verification must pass on the migrated writer");
    assert!(spawned, "a configured (lazy) reader must spawn the probe");

    drop_scratch_db(&admin, db.pool.clone(), &wname).await;
}

/// Thread replies: head fetch reads the writer; a FULL cursor page is
/// served by the replica; an UNDER-limit cursor page (candidate terminal
/// page) is re-run on the writer so a lagged replica can never truncate
/// the tail into a false EOF.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_replies_cursor_pages_route_to_replica_with_writer_terminal_verification() {
    let admin = PgPool::connect(&admin_url().await)
        .await
        .expect("connect admin");
    let (writer, wname) = create_scratch_db(&admin, "routing_tw").await;
    let (replica, rname) = create_scratch_db(&admin, "routing_tr").await;

    let author = nostr::Keys::generate();
    let community = Uuid::new_v4();
    let channel = Uuid::new_v4();
    seed_community_channel(&writer, community, channel, &author).await;
    seed_community_channel(&replica, community, channel, &author).await;

    let base = 1_700_000_000u64;
    let root = signed_event_at(&author, "root", base);
    for pool in [&writer, &replica] {
        insert_top_level(pool, community, channel, &root).await;
    }

    // Writer holds replies r1..r5; the lagged replica only has r1..r3.
    let replies: Vec<nostr::Event> = (1..=5)
        .map(|i| signed_event_at(&author, &format!("r{i}"), base + 10 * i as u64))
        .collect();
    for reply in &replies {
        insert_thread_reply(&writer, community, channel, &root, reply).await;
    }
    for reply in &replies[..3] {
        insert_thread_reply(&replica, community, channel, &root, reply).await;
    }

    let db = Db::from_pools(writer.clone(), replica.clone());
    // Open the fence through "now" — fixture history is far in the past.
    db.fence().force_open_for_tests(chrono::Utc::now());
    let cid = CommunityId::from_uuid(community);

    // Page 1 (no cursor) → writer.
    let page1 = db
        .get_thread_replies(cid, root.id.as_bytes(), Some(10), 2, None)
        .await
        .expect("page 1");
    let contents: Vec<&str> = page1
        .iter()
        .map(|r| r.stored_event.event.content.as_str())
        .collect();
    assert_eq!(contents, vec!["r1", "r2"], "head page from writer");

    // Page 2: replica serves a FULL page (r3 exists there) — but wait:
    // replica has r1..r3, page after r2 with limit 2 returns only [r3]
    // (under limit) → terminal-verification re-runs on the writer, which
    // returns [r3, r4]. A lag-truncated EOF must never surface.
    let cur2 = thread_cursor(page1.last().expect("page 1 non-empty"));
    let page2 = db
        .get_thread_replies(cid, root.id.as_bytes(), Some(10), 2, Some(&cur2))
        .await
        .expect("page 2");
    let contents: Vec<&str> = page2
        .iter()
        .map(|r| r.stored_event.event.content.as_str())
        .collect();
    assert_eq!(
        contents,
        vec!["r3", "r4"],
        "under-limit replica page must be re-verified on the writer"
    );

    // Full-page replica serve: with limit 1, the page after r2 is [r3] —
    // exactly `limit` rows, so the replica result stands. Prove it came
    // from the replica with a replica-only divergent reply.
    let ghost = signed_event_at(&author, "replica-only-ghost", base + 25);
    insert_thread_reply(&replica, community, channel, &root, &ghost).await;
    let page_replica = db
        .get_thread_replies(cid, root.id.as_bytes(), Some(10), 1, Some(&cur2))
        .await
        .expect("full replica page");
    let contents: Vec<&str> = page_replica
        .iter()
        .map(|r| r.stored_event.event.content.as_str())
        .collect();
    assert_eq!(
        contents,
        vec!["replica-only-ghost"],
        "a full cursor page must be served by the replica"
    );

    // Same query with no replica configured reads the writer and cannot
    // see the ghost.
    let db_writer_only = Db::from_pool(writer.clone());
    let page_writer = db_writer_only
        .get_thread_replies(cid, root.id.as_bytes(), Some(10), 1, Some(&cur2))
        .await
        .expect("writer-only page");
    let contents: Vec<&str> = page_writer
        .iter()
        .map(|r| r.stored_event.event.content.as_str())
        .collect();
    assert_eq!(contents, vec!["r3"], "unset replica falls back to writer");

    drop_scratch_db(&admin, replica, &rname).await;
    drop_scratch_db(&admin, writer, &wname).await;
}

/// Channel DESC scrollback, out-of-order commit adversary: the replica is
/// missing a MIDDLE row (`m2`) because a transaction with an older
/// client-signed `created_at` committed late and has not replayed yet.
/// The replica's cursor page would be `[m1]` — silently skipping `m2`
/// forever, since the next cursor advances past it. The fence must route
/// any cursor above it to the writer.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn channel_cursor_above_fence_stays_on_writer_preventing_middle_hole() {
    let admin = PgPool::connect(&admin_url().await)
        .await
        .expect("connect admin");
    let (writer, wname) = create_scratch_db(&admin, "fence_cw").await;
    let (replica, rname) = create_scratch_db(&admin, "fence_cr").await;

    let author = nostr::Keys::generate();
    let community = Uuid::new_v4();
    let channel = Uuid::new_v4();
    seed_community_channel(&writer, community, channel, &author).await;
    seed_community_channel(&replica, community, channel, &author).await;

    let base = 1_700_000_000u64;
    let m1 = signed_event_at(&author, "m1", base);
    let m2 = signed_event_at(&author, "m2-late-commit", base + 10);
    let m3 = signed_event_at(&author, "m3", base + 20);
    let m4 = signed_event_at(&author, "m4", base + 30);
    for ev in [&m1, &m2, &m3, &m4] {
        insert_top_level(&writer, community, channel, ev).await;
    }
    // Replica replayed everything EXCEPT the late-committed m2.
    for ev in [&m1, &m3, &m4] {
        insert_top_level(&replica, community, channel, ev).await;
    }

    let db = Db::from_pools(writer.clone(), replica.clone());
    let cid = CommunityId::from_uuid(community);

    // Head page (writer): [m4, m3]; cursor lands on m3 (base+20).
    let head = db
        .get_channel_window(cid, channel, 2, None, None)
        .await
        .expect("head window");
    let cursor = head.next_cursor.expect("has_more implies next_cursor");

    // Fence closed → cursor page must come from the writer: m2 present.
    let contents = |w: &thread::ChannelWindow| -> Vec<String> {
        w.rows
            .iter()
            .map(|r| r.stored_event.event.content.clone())
            .collect()
    };
    let page_closed = db
        .get_channel_window(cid, channel, 10, Some(cursor.clone()), None)
        .await
        .expect("cursor page, fence closed");
    assert_eq!(
        contents(&page_closed),
        vec!["m2-late-commit".to_string(), "m1".to_string()],
        "fence closed: cursor pages route to the writer"
    );

    // Fence open but BELOW the cursor timestamp (covers base+5 only):
    // the cursor (base+20) is not covered → writer again.
    db.fence()
        .force_open_for_tests(chrono::DateTime::from_timestamp(base as i64 + 5, 0).expect("ts"));
    let page_below = db
        .get_channel_window(cid, channel, 10, Some(cursor.clone()), None)
        .await
        .expect("cursor page, fence below cursor");
    assert_eq!(
        contents(&page_below),
        vec!["m2-late-commit".to_string(), "m1".to_string()],
        "cursor above the fence must stay on the writer"
    );

    // Counterfactual pinning the hazard: were the fence (wrongly) open
    // through now, the replica would serve the page WITHOUT m2 — the
    // permanent-skip hole this fence exists to prevent.
    db.fence().force_open_for_tests(chrono::Utc::now());
    let page_hazard = db
        .get_channel_window(cid, channel, 10, Some(cursor), None)
        .await
        .expect("cursor page, fence wrongly open");
    assert_eq!(
        contents(&page_hazard),
        vec!["m1".to_string()],
        "fixture models the inversion: an over-open fence would skip m2"
    );

    drop_scratch_db(&admin, replica, &rname).await;
    drop_scratch_db(&admin, writer, &wname).await;
}

/// Thread ASC pagination, out-of-order commit adversary: the replica
/// holds a FULL page whose newest row (`r4`) has a later key than a
/// not-yet-replayed row (`r3`). The old under-limit check alone would
/// serve `[r4]` and the client cursor would advance past `r3` forever.
/// The fence rule (full AND tail ≤ fence) must send that page to the
/// writer instead.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_full_replica_page_above_fence_is_reverified_on_writer() {
    let admin = PgPool::connect(&admin_url().await)
        .await
        .expect("connect admin");
    let (writer, wname) = create_scratch_db(&admin, "fence_tw").await;
    let (replica, rname) = create_scratch_db(&admin, "fence_tr").await;

    let author = nostr::Keys::generate();
    let community = Uuid::new_v4();
    let channel = Uuid::new_v4();
    seed_community_channel(&writer, community, channel, &author).await;
    seed_community_channel(&replica, community, channel, &author).await;

    let base = 1_700_000_000u64;
    let root = signed_event_at(&author, "root", base);
    for pool in [&writer, &replica] {
        insert_top_level(pool, community, channel, &root).await;
    }
    let replies: Vec<nostr::Event> = (1..=4)
        .map(|i| signed_event_at(&author, &format!("r{i}"), base + 10 * i as u64))
        .collect();
    for reply in &replies {
        insert_thread_reply(&writer, community, channel, &root, reply).await;
    }
    // Replica replayed r1, r2, r4 — the late-committed r3 is missing.
    for reply in [&replies[0], &replies[1], &replies[3]] {
        insert_thread_reply(&replica, community, channel, &root, reply).await;
    }

    let db = Db::from_pools(writer.clone(), replica.clone());
    let cid = CommunityId::from_uuid(community);

    // Fence covers r2 (base+20) but not r3/r4.
    db.fence()
        .force_open_for_tests(chrono::DateTime::from_timestamp(base as i64 + 20, 0).expect("ts"));

    // Page after r2 with limit 1: the replica would return the FULL page
    // [r4] — but its tail is above the fence, so the writer re-runs it
    // and returns [r3]. No skip.
    let page1 = db
        .get_thread_replies(cid, root.id.as_bytes(), Some(10), 2, None)
        .await
        .expect("head page");
    let cur = thread_cursor(page1.last().expect("head page non-empty"));
    let page = db
        .get_thread_replies(cid, root.id.as_bytes(), Some(10), 1, Some(&cur))
        .await
        .expect("cursor page");
    let contents: Vec<&str> = page
        .iter()
        .map(|r| r.stored_event.event.content.as_str())
        .collect();
    assert_eq!(
        contents,
        vec!["r3"],
        "a full replica page above the fence must be re-run on the writer"
    );

    // Counterfactual: an over-open fence would serve the replica's [r4],
    // skipping r3 permanently.
    db.fence().force_open_for_tests(chrono::Utc::now());
    let hazard = db
        .get_thread_replies(cid, root.id.as_bytes(), Some(10), 1, Some(&cur))
        .await
        .expect("hazard page");
    let contents: Vec<&str> = hazard
        .iter()
        .map(|r| r.stored_event.event.content.as_str())
        .collect();
    assert_eq!(
        contents,
        vec!["r4"],
        "fixture models the inversion: an over-open fence would skip r3"
    );

    drop_scratch_db(&admin, replica, &rname).await;
    drop_scratch_db(&admin, writer, &wname).await;
}

/// Commit-time floor guard (migration 0021), exact held-transaction
/// adversary: a channel-bearing row whose `created_at` is older than the
/// floor at COMMIT time must abort the transaction — the guard runs
/// inside commit processing with `clock_timestamp()`, so holding the
/// transaction open cannot outrun it. channel_id-NULL rows are
/// structurally exempt, and sessions without the GUC are unaffected.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn created_at_floor_guard_aborts_old_channel_rows_at_commit() {
    let admin = PgPool::connect(&admin_url().await)
        .await
        .expect("connect admin");
    let (pool, name) = create_scratch_db(&admin, "floor_guard").await;

    let author = nostr::Keys::generate();
    let community = Uuid::new_v4();
    let channel = Uuid::new_v4();
    seed_community_channel(&pool, community, channel, &author).await;

    let insert_raw = |ev: nostr::Event, channel_id: Option<Uuid>| {
        let pool = pool.clone();
        async move {
            let mut tx = pool.begin().await.expect("begin");
            // Arm the guard for this transaction only (the relay's
            // writer pool arms it per connection; tests are explicit).
            sqlx::query("SELECT set_config('buzz.created_at_floor', $1, true)")
                .bind(crate::replica_fence::CREATED_AT_FLOOR_SECS.to_string())
                .execute(&mut *tx)
                .await
                .expect("arm guard");
            sqlx::query(
                "INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, \
                     content, sig, received_at, channel_id) \
                     VALUES ($1, $2, $3, to_timestamp($4), 9, '[]', $5, $6, NOW(), $7)",
            )
            .bind(community)
            .bind(ev.id.as_bytes().as_slice())
            .bind(ev.pubkey.to_bytes().as_slice())
            .bind(ev.created_at.as_secs() as f64)
            .bind(&ev.content)
            .bind(ev.sig.serialize().as_slice())
            .bind(channel_id)
            .execute(&mut *tx)
            .await
            .expect("insert inside tx (guard is deferred to commit)");
            // Hold the transaction "open" past the insert, then commit —
            // the deferred guard must still see the stale created_at.
            sqlx::query("SELECT pg_sleep(0.05)")
                .execute(&mut *tx)
                .await
                .expect("hold tx");
            tx.commit().await
        }
    };

    let now_secs = chrono::Utc::now().timestamp() as u64;
    let floor = crate::replica_fence::CREATED_AT_FLOOR_SECS as u64;

    // Old channel-bearing row → COMMIT aborts with check_violation.
    let old = signed_event_at(&author, "old-held-tx", now_secs - floor - 60);
    let err = insert_raw(old, Some(channel))
        .await
        .expect_err("below-floor channel row must abort at COMMIT");
    let code = match &err {
        sqlx::Error::Database(db_err) => db_err.code().map(|c| c.to_string()),
        other => panic!("expected database error, got {other:?}"),
    };
    assert_eq!(
        code.as_deref(),
        Some("23514"),
        "guard raises check_violation"
    );

    // Fresh channel-bearing row → commits.
    let fresh = signed_event_at(&author, "fresh", now_secs);
    insert_raw(fresh, Some(channel))
        .await
        .expect("fresh row commits under the armed guard");

    // Old row WITHOUT a channel (push lease / profile shapes) →
    // structurally exempt, commits.
    let old_global = signed_event_at(&author, "old-global", now_secs - floor - 60);
    insert_raw(old_global, None)
        .await
        .expect("channel_id-NULL rows are exempt from the floor");

    // Unarmed session (no GUC) → guard inert; backfills stay possible
    // (and must hold the fence closed, per the migration header).
    let old_backfill = signed_event_at(&author, "old-backfill", now_secs - floor - 60);
    insert_top_level(&pool, community, channel, &old_backfill).await;

    drop_scratch_db(&admin, pool, &name).await;
}

#[test]
fn writer_pool_safety_hook_is_single_and_composed() {
    let source = include_str!("mod.rs");
    let connect_pool = source
        .split("async fn connect_writer_pool")
        .nth(1)
        .and_then(|tail| tail.split("const READER_ACQUIRE_TIMEOUT").next())
        .expect("connect_writer_pool source block");
    assert_eq!(
        connect_pool.matches(".after_connect(").count(),
        1,
        "SQLx replaces after_connect hooks; writer safety must use exactly one"
    );
    assert!(connect_pool.contains("buzz.created_at_floor"));
    assert!(connect_pool.contains("SHOW transaction_isolation"));
    assert!(connect_pool.contains("'lock_timeout'"));
    assert!(connect_pool.contains("'idle_in_transaction_session_timeout'"));
    assert!(connect_pool.contains("'statement_timeout'"));
    assert!(!connect_pool.contains("arm_floor_guard"));
    assert!(!connect_pool.contains("_arm_floor_guard"));
    assert!(!connect_pool.contains("allow(unused_variables)"));

    let reader_doc = source
        .split("fn connect_read_pool")
        .next()
        .and_then(|prefix| prefix.rsplit("/// Connect the read-replica").next())
        .expect("reader pool documentation");
    assert!(reader_doc.contains("replica sessions are"));
    assert!(reader_doc.contains("read-only"));
    assert!(!reader_doc.contains("Db::connect_writer_pool"));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn writer_pool_rejects_non_read_committed_database_default() {
    let admin = PgPool::connect(&admin_url().await)
        .await
        .expect("connect admin");
    let (seed_pool, name) = create_scratch_db(&admin, "writer_isolation").await;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "ALTER DATABASE {name} SET default_transaction_isolation = 'repeatable read'"
    )))
    .execute(&admin)
    .await
    .expect("set unsafe database default");
    seed_pool.close().await;

    let base = admin_url().await;
    let idx = base.rfind('/').expect("db url has a path segment");
    let scratch_url = format!("{}/{}", &base[..idx], name);
    let error = Db::new(&DbConfig {
        database_url: scratch_url,
        max_connections: 1,
        min_connections: 1,
        acquire_timeout_secs: 1,
        ..DbConfig::default()
    })
    .await
    .expect_err("writer pool must reject pinned-snapshot database defaults");
    assert!(
        error.to_string().contains("requires READ COMMITTED")
            || error.to_string().contains("pool timed out"),
        "unexpected isolation rejection: {error}"
    );

    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE {name} WITH (FORCE)"
    )))
    .execute(&admin)
    .await
    .expect("drop isolation test database");
}

/// Session-timeout environment overrides retain PostgreSQL's `0 = disabled`
/// semantics and ignore invalid values.
#[test]
fn session_timeout_env_overlay_zero_passthrough_and_invalid_fallback() {
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = ENV_LOCK.lock().unwrap();
    let keys = [
        "BUZZ_DB_LOCK_TIMEOUT_MS",
        "BUZZ_DB_IDLE_TXN_TIMEOUT_MS",
        "BUZZ_DB_STATEMENT_TIMEOUT_MS",
    ];
    let previous: Vec<_> = keys.iter().map(std::env::var_os).collect();
    let read = |config: DbConfig| {
        (
            config.lock_timeout_ms,
            config.idle_txn_timeout_ms,
            config.statement_timeout_ms,
        )
    };

    for key in keys {
        std::env::remove_var(key);
    }
    let unset = read(DbConfig::default().with_session_timeouts_from_env());

    std::env::set_var("BUZZ_DB_LOCK_TIMEOUT_MS", "2000");
    std::env::set_var("BUZZ_DB_IDLE_TXN_TIMEOUT_MS", "30000");
    std::env::set_var("BUZZ_DB_STATEMENT_TIMEOUT_MS", "10000");
    let overridden = read(DbConfig::default().with_session_timeouts_from_env());

    for key in keys {
        std::env::set_var(key, "0");
    }
    let zero = read(DbConfig::default().with_session_timeouts_from_env());

    for key in keys {
        std::env::set_var(key, "not-a-number");
    }
    let junk = read(DbConfig::default().with_session_timeouts_from_env());

    for (key, value) in keys.iter().zip(previous) {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    let defaults = (DEFAULT_LOCK_TIMEOUT_MS, DEFAULT_IDLE_TXN_TIMEOUT_MS, 0);
    assert_eq!(unset, defaults, "unset env must keep the defaults");
    assert_eq!(overridden, (2000, 30000, 10000));
    assert_eq!(zero, (0, 0, 0), "explicit 0 must disable each timeout");
    assert_eq!(junk, defaults, "junk env must keep the defaults");
}

/// The production writer constructor installs all three timeout GUCs, bounds
/// ordinary lock waits, and exempts the intentional migration lock wait.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn session_timeouts_install_through_db_new_and_bound_lock_waits() {
    let admin = PgPool::connect(&admin_url().await)
        .await
        .expect("connect admin");
    let (seed_pool, name) = create_scratch_db(&admin, "session_timeouts").await;
    seed_pool.close().await;

    let base = admin_url().await;
    let idx = base.rfind('/').expect("db url has a path segment");
    let scratch_url = format!("{}/{}", &base[..idx], name);
    let db = Db::new(&DbConfig {
        database_url: scratch_url.clone(),
        max_connections: 2,
        lock_timeout_ms: 500,
        idle_txn_timeout_ms: 60_000,
        statement_timeout_ms: 0,
        ..DbConfig::default()
    })
    .await
    .expect("connect Db with session timeouts");

    let (lock, idle, statement): (String, String, String) = sqlx::query_as(
        "SELECT current_setting('lock_timeout'), \
                current_setting('idle_in_transaction_session_timeout'), \
                current_setting('statement_timeout')",
    )
    .fetch_one(&db.pool)
    .await
    .expect("read effective GUCs");
    assert_eq!(lock, "500ms");
    assert_eq!(idle, "1min");
    assert_eq!(statement, "0");

    let mut holder = db.pool.acquire().await.expect("holder connection");
    sqlx::raw_sql("BEGIN; LOCK TABLE events IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *holder)
        .await
        .expect("hold relation lock");
    let waited = std::time::Instant::now();
    let mut waiter_txn = db.pool.begin().await.expect("waiter transaction");
    let error = sqlx::query("LOCK TABLE events IN ACCESS SHARE MODE")
        .execute(&mut *waiter_txn)
        .await
        .expect_err("waiter must time out, not park");
    drop(waiter_txn);
    let code = match &error {
        sqlx::Error::Database(db_error) => db_error.code().map(|code| code.to_string()),
        other => panic!("expected database error, got {other:?}"),
    };
    assert_eq!(code.as_deref(), Some("55P03"));
    assert!(waited.elapsed() < std::time::Duration::from_secs(5));

    let mut advisory_holder = PgPool::connect(&scratch_url)
        .await
        .expect("advisory holder pool")
        .acquire()
        .await
        .expect("advisory holder conn")
        .detach();
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(crate::deletion::SCHEMA_DESTRUCTION_LOCK_KEY)
        .execute(&mut advisory_holder)
        .await
        .expect("hold schema advisory lock");
    let release = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(1_500)).await;
        let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
            .bind(crate::deletion::SCHEMA_DESTRUCTION_LOCK_KEY)
            .execute(&mut advisory_holder)
            .await;
        let _ = advisory_holder.close().await;
    });
    db.migrate()
        .await
        .expect("migrate must wait out the advisory holder");
    release.await.expect("release task");

    let _ = sqlx::query("ROLLBACK").execute(&mut *holder).await;
    drop(holder);
    drop_scratch_db(&admin, db.pool.clone(), &name).await;
}

/// The armed writer pool (`Db::new`) must enforce the floor end-to-end
/// through the public insert APIs, and the session GUC must be verifiably
/// set on pooled connections.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn armed_pool_rejects_old_channel_inserts_through_public_api() {
    let admin = PgPool::connect(&admin_url().await)
        .await
        .expect("connect admin");
    let (seed_pool, name) = create_scratch_db(&admin, "floor_pool").await;

    let author = nostr::Keys::generate();
    let community = Uuid::new_v4();
    let channel = Uuid::new_v4();
    seed_community_channel(&seed_pool, community, channel, &author).await;

    // Connect a Db the production way: after_connect arms the guard.
    let base = admin_url().await;
    let idx = base.rfind('/').expect("db url has a path segment");
    let scratch_url = format!("{}/{}", &base[..idx], name);
    let db = Db::new(&DbConfig {
        database_url: scratch_url,
        max_connections: 2,
        ..DbConfig::default()
    })
    .await
    .expect("connect armed Db");
    let cid = CommunityId::from_uuid(community);

    // Perci nit: assert the effective session value, not the intent.
    let effective: String = sqlx::query_scalar("SHOW buzz.created_at_floor")
        .fetch_one(&db.pool)
        .await
        .expect("SHOW guard GUC");
    assert_eq!(
        effective,
        crate::replica_fence::CREATED_AT_FLOOR_SECS.to_string(),
        "writer pool must arm the floor guard on every connection"
    );
    let isolation: String = sqlx::query_scalar("SHOW transaction_isolation")
        .fetch_one(&db.pool)
        .await
        .expect("SHOW writer isolation");
    assert_eq!(
        isolation, "read committed",
        "the same writer after_connect hook must enforce the isolation premise"
    );

    let now_secs = chrono::Utc::now().timestamp() as u64;
    let floor = crate::replica_fence::CREATED_AT_FLOOR_SECS as u64;

    // insert_event (single INSERT, autocommit): old channel row rejected.
    let old = signed_event_at(&author, "old-direct", now_secs - floor - 60);
    let err = event::insert_event(&db.pool, cid, &old, Some(channel))
        .await
        .expect_err("armed pool must reject below-floor channel inserts");
    assert!(
        err.to_string().contains("below the replica-fence floor"),
        "unexpected error: {err}"
    );

    // insert_event_with_thread_metadata (multi-statement tx): same.
    let old2 = signed_event_at(&author, "old-thread-meta", now_secs - floor - 90);
    let ts =
        chrono::DateTime::from_timestamp(old2.created_at.as_secs() as i64, 0).expect("valid ts");
    let err = event::insert_event_with_thread_metadata(
        &db.pool,
        cid,
        &old2,
        Some(channel),
        Some(event::ThreadMetadataParams {
            event_id: old2.id.as_bytes(),
            event_created_at: ts,
            channel_id: channel,
            parent_event_id: None,
            parent_event_created_at: None,
            root_event_id: None,
            root_event_created_at: None,
            depth: 0,
            broadcast: true,
        }),
    )
    .await
    .expect_err("armed pool must reject below-floor thread-metadata inserts");
    assert!(
        err.to_string().contains("below the replica-fence floor"),
        "unexpected error: {err}"
    );

    // Fresh events pass through both APIs.
    let fresh = signed_event_at(&author, "fresh-direct", now_secs);
    event::insert_event(&db.pool, cid, &fresh, Some(channel))
        .await
        .expect("fresh insert passes the armed guard");

    drop_scratch_db(&admin, seed_pool, &name).await;
    // db pool still holds connections to the dropped DB; close it.
    db.pool.close().await;
}

/// `spawn_fence_probe` must verify the floor guard before letting the
/// probe run — catalog shape AND observed behavior — and refuse on
/// sabotage. This is the production gate for a relay running with
/// `BUZZ_AUTO_MIGRATE` off: an armed GUC with no enforcing trigger must
/// never yield an open fence.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn fence_probe_refuses_to_start_without_verified_floor_guard() {
    let admin = PgPool::connect(&admin_url().await)
        .await
        .expect("connect admin");
    let (seed_pool, wname) = create_scratch_db(&admin, "fence_gate_w").await;
    let (replica_pool, rname) = create_scratch_db(&admin, "fence_gate_r").await;
    seed_pool.close().await;
    replica_pool.close().await;

    let base = admin_url().await;
    let idx = base.rfind('/').expect("db url has a path segment");
    let writer_url = format!("{}/{}", &base[..idx], wname);
    let replica_url = format!("{}/{}", &base[..idx], rname);

    // Healthy schema: verification passes, probe starts. A SEPARATE Db
    // instance, because its background probe legitimately opens its own
    // fence (the heartbeat probe is writer-side only) — the refusal
    // assertions below must run against a fence whose spawns were all
    // refused.
    let db_healthy = Db::new(&DbConfig {
        database_url: writer_url.clone(),
        read_database_url: Some(replica_url.clone()),
        max_connections: 2,
        ..DbConfig::default()
    })
    .await
    .expect("connect armed Db with replica");
    assert!(
        db_healthy
            .spawn_fence_probe()
            .await
            .expect("verification passes"),
        "probe must start on a verified schema"
    );

    let db = Db::new(&DbConfig {
        database_url: writer_url,
        read_database_url: Some(replica_url),
        max_connections: 2,
        ..DbConfig::default()
    })
    .await
    .expect("connect armed Db with replica");

    // Sabotage A: catalog-shaped no-op — same trigger, gutted function
    // body. Catalog check alone would pass; behavior check must refuse.
    sqlx::query(
        "CREATE OR REPLACE FUNCTION events_created_at_floor_guard() RETURNS trigger \
             LANGUAGE plpgsql AS $$ BEGIN RETURN NULL; END $$",
    )
    .execute(&db.pool)
    .await
    .expect("gut the guard function");
    let err = db
        .spawn_fence_probe()
        .await
        .expect_err("inert guard body must refuse the probe");
    assert!(
        err.to_string().contains("floor guard is inert"),
        "unexpected error: {err}"
    );

    // Sabotage B: trigger dropped entirely (the BUZZ_AUTO_MIGRATE=off /
    // 0021-unapplied shape). Catalog check must refuse.
    sqlx::query("DROP TRIGGER events_created_at_floor ON events")
        .execute(&db.pool)
        .await
        .expect("drop the guard trigger");
    let err = db
        .spawn_fence_probe()
        .await
        .expect_err("missing trigger must refuse the probe");
    assert!(
        err.to_string().contains("missing or mis-shaped"),
        "unexpected error: {err}"
    );

    // In both refusal states the fence never opened.
    assert!(
        db.fence().verified_through().is_none(),
        "fence must remain closed when verification refuses the probe"
    );

    db_healthy.pool.close().await;
    if let Some(rp) = &db_healthy.read_pool {
        rp.close().await;
    }
    db.pool.close().await;
    if let Some(rp) = &db.read_pool {
        rp.close().await;
    }
    let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE IF EXISTS {wname} WITH (FORCE)"
    )))
    .execute(&admin)
    .await;
    let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE IF EXISTS {rname} WITH (FORCE)"
    )))
    .execute(&admin)
    .await;
}

/// The `UPDATE OF` arm of the floor guard (Perci's second structural
/// hole): an old row legitimately admitted with `channel_id` NULL must
/// not be movable into keyset windows, and a channel row's `created_at`
/// must not be movable below the fence — through raw SQL, at COMMIT.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn floor_guard_blocks_updates_that_move_rows_below_the_fence() {
    let admin = PgPool::connect(&admin_url().await)
        .await
        .expect("connect admin");
    let (pool, name) = create_scratch_db(&admin, "floor_upd").await;

    let author = nostr::Keys::generate();
    let community = Uuid::new_v4();
    let channel = Uuid::new_v4();
    seed_community_channel(&pool, community, channel, &author).await;

    let now_secs = chrono::Utc::now().timestamp() as u64;
    let floor = crate::replica_fence::CREATED_AT_FLOOR_SECS as u64;

    // Seed via unarmed session: one old channel-NULL row, one fresh
    // channel row.
    let old_null = signed_event_at(&author, "old-null", now_secs - floor - 120);
    insert_top_level(&pool, community, channel, &old_null).await;
    sqlx::query("UPDATE events SET channel_id = NULL WHERE community_id = $1 AND id = $2")
        .bind(community)
        .bind(old_null.id.as_bytes().as_slice())
        .execute(&pool)
        .await
        .expect("detach channel (unarmed seed)");
    let fresh = signed_event_at(&author, "fresh-row", now_secs);
    insert_top_level(&pool, community, channel, &fresh).await;

    // Armed transaction, deferred to COMMIT (the production shape).
    let run_armed_update = |sql: &'static str, id: Vec<u8>, age: Option<u64>| {
        let pool = pool.clone();
        async move {
            let mut tx = pool.begin().await.expect("begin");
            sqlx::query("SELECT set_config('buzz.created_at_floor', $1, true)")
                .bind(crate::replica_fence::CREATED_AT_FLOOR_SECS.to_string())
                .execute(&mut *tx)
                .await
                .expect("arm guard");
            let q = sqlx::query(sql).bind(community).bind(id);
            let q = match age {
                Some(a) => q.bind(a as f64),
                None => q,
            };
            q.execute(&mut *tx)
                .await
                .expect("update inside tx (deferred)");
            tx.commit().await
        }
    };

    // channel-NULL → channel-bearing on an old row: COMMIT must abort.
    let err = run_armed_update(
        "UPDATE events SET channel_id = community_id WHERE community_id = $1 AND id = $2",
        old_null.id.as_bytes().to_vec(),
        None,
    )
    .await
    .expect_err("moving an old channel-NULL row into a channel must abort at COMMIT");
    assert!(
        matches!(&err, sqlx::Error::Database(e) if e.code().as_deref() == Some("23514")),
        "unexpected error: {err}"
    );

    // created_at rewrite below the floor on a channel row: COMMIT must abort.
    let err = run_armed_update(
            "UPDATE events SET created_at = clock_timestamp() - make_interval(secs => $3::double precision) \
             WHERE community_id = $1 AND id = $2",
            fresh.id.as_bytes().to_vec(),
            Some(floor + 120),
        )
        .await
        .expect_err("rewriting created_at below the floor must abort at COMMIT");
    assert!(
        matches!(&err, sqlx::Error::Database(e) if e.code().as_deref() == Some("23514")),
        "unexpected error: {err}"
    );

    drop_scratch_db(&admin, pool, &name).await;
}
