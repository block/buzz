//! Reader-pool warm floor and fence-proof bounds, against a real Postgres
//! standing in for the replica (the scratch writer database doubles as the
//! reader).

use super::*;
use std::time::Instant;

/// A `Db` whose writer and lazy reader both point at one scratch database,
/// with the fence open so routed reads attempt the reader.
async fn same_db_reader(prefix: &str, read_max: u32) -> (PgPool, Db, String, String) {
    let admin = PgPool::connect(&admin_url().await)
        .await
        .expect("connect admin");
    let (seed, wname) = create_scratch_db(&admin, prefix).await;
    seed.close().await;
    let scratch_url = {
        let base = admin_url().await;
        let idx = base.rfind('/').expect("db url has a path segment");
        format!("{}/{}", &base[..idx], wname)
    };
    let mut db = Db::new(&DbConfig {
        database_url: scratch_url.clone(),
        read_database_url: Some(scratch_url.clone()),
        max_connections: 4,
        read_max_connections: Some(read_max),
        ..DbConfig::default()
    })
    .await
    .expect("connect Db with lazy reader");
    db.fence().force_open_for_tests(chrono::Utc::now());
    db.set_replica_read_max_age_for_tests(Some(Duration::from_secs(30)));
    (admin, db, wname, scratch_url)
}

/// Poll until the reader pool holds exactly `target` connections, all idle.
/// Returned and floor-maintenance connections land asynchronously, from
/// tasks sqlx spawns on drop.
async fn await_idle_floor(read_pool: &PgPool, target: u32) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while read_pool.size() != target || read_pool.num_idle() != target as usize {
        assert!(
            Instant::now() < deadline,
            "reader pool never settled at {target} idle; size={} idle={}",
            read_pool.size(),
            read_pool.num_idle()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// `buzz_db_route_decision` counters keyed by `(decision, reason)`.
fn route_reasons(snapshotter: &Snapshotter) -> std::collections::HashMap<(String, String), u64> {
    snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .filter(|(key, ..)| key.key().name() == "buzz_db_route_decision")
        .map(|(key, _, _, value)| {
            let DebugValue::Counter(n) = value else {
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
        .collect()
}

/// The boot ping fills the reader pool to its warm floor, so a freshly
/// started pod meets its first routed reads with established connections
/// instead of dialing each one inside the 150ms reader acquire budget.
/// Without the warm-up the ping leaves exactly one connection behind.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn reader_boot_ping_warms_pool_to_min_connections() {
    let (admin, db, wname, _) = same_db_reader("warm_r", 15).await;
    let read_pool = db.read_pool.clone().expect("reader pool configured");
    let target = Db::reader_min_connections(15);
    assert_eq!(
        target, 3,
        "fixture needs a floor above the ping's one connection"
    );
    assert_eq!(
        read_pool.size(),
        0,
        "lazy reader dials nothing before the ping"
    );

    db.read_pool_boot_ping_for_tests().await;

    await_idle_floor(&read_pool, target).await;
    // Settled at the floor and no further: hold briefly and re-check that
    // maintenance does not keep dialing.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(read_pool.size(), target, "warm-up must stop at the floor");

    drop_scratch_db(&admin, db.pool.clone(), &wname).await;
}

/// A warm reader connection whose server stops answering (here: the
/// heartbeat observation blocks behind an exclusive lock) must fail the
/// routed read closed to the writer within the proof budget, not hang the
/// request. Without a pre-acquire ping, the proof is the first round trip
/// on the checkout and carries the liveness bound.
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires Postgres"]
async fn routed_read_proof_times_out_to_writer_on_unresponsive_reader() {
    let (admin, db, wname, scratch_url) = same_db_reader("proof_to", 5).await;
    let read_pool = db.read_pool.clone().expect("reader pool configured");
    db.read_pool_boot_ping_for_tests().await;
    await_idle_floor(&read_pool, Db::reader_min_connections(5)).await;

    let mut locker = sqlx::PgConnection::connect(&scratch_url)
        .await
        .expect("connect locker");
    sqlx::query("BEGIN").execute(&mut locker).await.unwrap();
    sqlx::query("LOCK TABLE replica_heartbeat IN ACCESS EXCLUSIVE MODE")
        .execute(&mut locker)
        .await
        .expect("lock heartbeat");

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let query = EventQuery::for_community(CommunityId::from_uuid(Uuid::new_v4()));
    let start = Instant::now();
    let count = {
        let _guard = metrics::set_default_local_recorder(&recorder);
        tokio::time::timeout(
            Duration::from_secs(10),
            db.count_events_routed("proof_timeout_probe", &query),
        )
        .await
        .expect("routed read must not hang on an unresponsive reader")
    }
    .expect("writer fallback still answers the read");
    let elapsed = start.elapsed();

    assert_eq!(count, 0, "writer answered on an empty scratch database");
    assert!(
        elapsed >= Db::READER_PROOF_TIMEOUT && elapsed < Db::READER_PROOF_TIMEOUT * 3,
        "fallback must wait out one proof budget ({}ms) and no more; took {}ms",
        Db::READER_PROOF_TIMEOUT.as_millis(),
        elapsed.as_millis()
    );
    let reasons = route_reasons(&snapshotter);
    assert_eq!(
        reasons.get(&("writer".to_owned(), "reader_proof_timeout".to_owned())),
        Some(&1),
        "unresponsive reader must fall back as writer/reader_proof_timeout; got {reasons:?}"
    );

    sqlx::query("ROLLBACK").execute(&mut locker).await.unwrap();
    locker.close().await.ok();
    drop_scratch_db(&admin, db.pool.clone(), &wname).await;
}

/// With no pre-acquire ping, a warm reader connection killed server-side is
/// handed out as-is; the routed read must still fail closed to the writer as
/// `reader_validation_error` with no error reaching the caller.
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires Postgres"]
async fn terminated_warm_reader_connection_fails_closed_to_writer() {
    let (admin, db, wname, _) = same_db_reader("term_r", 5).await;
    let read_pool = db.read_pool.clone().expect("reader pool configured");
    db.read_pool_boot_ping_for_tests().await;
    await_idle_floor(&read_pool, 1).await;

    let pid: i32 = {
        let mut conn = read_pool.acquire().await.expect("warm reader connection");
        sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *conn)
            .await
            .expect("reader backend pid")
    };
    await_idle_floor(&read_pool, 1).await;
    let terminated: bool = sqlx::query_scalar("SELECT pg_terminate_backend($1, 5000)")
        .bind(pid)
        .fetch_one(&admin)
        .await
        .expect("terminate warm reader backend");
    assert!(terminated, "warm reader backend must be terminated");

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let query = EventQuery::for_community(CommunityId::from_uuid(Uuid::new_v4()));
    let count = {
        let _guard = metrics::set_default_local_recorder(&recorder);
        db.count_events_routed("terminated_reader_probe", &query)
            .await
    }
    .expect("a dead reader connection must not surface as a client error");

    assert_eq!(count, 0, "writer answered on an empty scratch database");
    let reasons = route_reasons(&snapshotter);
    assert_eq!(
        reasons.get(&("writer".to_owned(), "reader_validation_error".to_owned())),
        Some(&1),
        "dead warm connection must fall back as writer/reader_validation_error; got {reasons:?}"
    );

    drop_scratch_db(&admin, db.pool.clone(), &wname).await;
}

/// A proof that times out while its `BEGIN` is still in flight must not
/// return the connection to the pool with a server-side transaction open.
/// sqlx counts the transaction only once `BEGIN` is acknowledged, so a
/// cancelled begin queues no `ROLLBACK`, and the release ping consumes the
/// late `BEGIN` reply: dropped instead of detached, the connection comes
/// back `idle in transaction` once the reader resumes.
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires Postgres"]
async fn proof_timeout_mid_begin_leaves_no_reader_transaction_open() {
    use sqlx::postgres::PgPoolOptions;

    let admin = PgPool::connect(&admin_url().await)
        .await
        .expect("connect admin");
    let (writer, wname) = create_scratch_db(&admin, "proof_tx").await;
    let writer_options = writer.connect_options();
    let proxy = crate::test_support::PausingProxy::spawn(
        writer_options.get_host().to_owned(),
        writer_options.get_port(),
    )
    .await;
    let read_pool = PgPoolOptions::new()
        .max_connections(5)
        .min_connections(1)
        .test_before_acquire(false)
        .acquire_timeout(Db::READER_ACQUIRE_TIMEOUT)
        .connect_lazy_with((*writer_options).clone().host("127.0.0.1").port(proxy.port));
    let mut db = Db::from_pools(writer.clone(), read_pool.clone());
    db.fence().force_open_for_tests(chrono::Utc::now());
    db.set_replica_read_max_age_for_tests(Some(Duration::from_secs(30)));
    // Primes the Aurora capability cache, so `BEGIN` is the proof's first
    // round trip, and warms the floor the routed read will check out.
    db.read_pool_boot_ping_for_tests().await;
    await_idle_floor(&read_pool, 1).await;

    proxy.set_paused(true);
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let query = EventQuery::for_community(CommunityId::from_uuid(Uuid::new_v4()));
    let count = {
        let _guard = metrics::set_default_local_recorder(&recorder);
        db.count_events_routed("proof_mid_begin_probe", &query)
            .await
    }
    .expect("writer fallback answers while the reader is paused");
    assert_eq!(count, 0, "writer answered on an empty scratch database");
    let reasons = route_reasons(&snapshotter);
    assert_eq!(
        reasons.get(&("writer".to_owned(), "reader_proof_timeout".to_owned())),
        Some(&1),
        "paused reader must fall back as writer/reader_proof_timeout; got {reasons:?}"
    );

    proxy.set_paused(false);
    await_idle_floor(&read_pool, 1).await;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let open: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity \
             WHERE datname = $1 AND state = 'idle in transaction'",
        )
        .bind(&wname)
        .fetch_one(&admin)
        .await
        .expect("read pg_stat_activity");
        if open == 0 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "{open} reader connection(s) returned to the pool idle in transaction"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    read_pool.close().await;
    drop_scratch_db(&admin, writer, &wname).await;
}
