//! Pool capacity must recover when old database sockets silently stop answering.
use super::*;
use crate::test_support::PausingProxy;
use sqlx::{pool::PoolConnection, ConnectOptions, Postgres};

/// Production pool budgets are sized for a warm relay pod, not for a test
/// lane whose first wave dials cold through [`PausingProxy`] while every other
/// worker clones its database. `connect_writer_pool` and `connect_with` dial
/// eagerly under `acquire_timeout`, so the budget must cover a loaded first
/// connection (the lane budget #9273 chose), not only a steady-state checkout.
const TEST_LANE_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(30);

async fn writer_pool(max: u32, min: u32) -> (PgPool, PausingProxy, ScratchDb) {
    let scratch = ScratchDb::create(&crate::test_support::database_url(), "release").await;
    let options: sqlx::postgres::PgConnectOptions = scratch.url().parse().unwrap();
    let proxy = PausingProxy::spawn(options.get_host().to_owned(), options.get_port()).await;
    let proxied = options
        .host("127.0.0.1")
        .port(proxy.port)
        .to_url_lossy()
        .to_string();
    let pool = Db::connect_writer_pool(&DbConfig {
        database_url: proxied,
        max_connections: max,
        min_connections: min,
        acquire_timeout_secs: TEST_LANE_ACQUIRE_TIMEOUT.as_secs(),
        ..DbConfig::default()
    })
    .await
    .expect("production writer pool");
    (pool, proxy, scratch)
}

async fn hold(pool: &PgPool, count: usize) -> Vec<PoolConnection<Postgres>> {
    let mut held = Vec::new();
    for _ in 0..count {
        let mut conn = pool.acquire().await.expect("checkout");
        sqlx::query("SELECT 1").execute(&mut *conn).await.unwrap();
        held.push(conn);
    }
    held
}

async fn await_capacity(pool: &PgPool, size: u32) {
    let started = std::time::Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if pool.size() == size && pool.num_idle() == size as usize {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    eprintln!(
        "release recovery: elapsed={:?} size={} idle={} target={size}",
        started.elapsed(),
        pool.size(),
        pool.num_idle()
    );
    result.expect("released connections must stop consuming pool capacity");
}

async fn fresh_query(pool: &PgPool) {
    let value: i32 = tokio::time::timeout(
        Duration::from_secs(5),
        sqlx::query_scalar("SELECT 1").fetch_one(pool),
    )
    .await
    .expect("fresh query deadline")
    .expect("fresh connection works while old sessions remain stalled");
    assert_eq!(value, 1);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn dropped_writer_connections_reclaim_saturated_pool() {
    let (pool, proxy, _scratch) = writer_pool(3, 0).await;
    let held = hold(&pool, 3).await;
    proxy.stall_established();
    drop(held);
    proxy.wait_for_held_sessions(3).await;
    await_capacity(&pool, 0).await;
    drop(hold(&pool, 3).await);
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn aborted_writer_queries_reclaim_saturated_pool() {
    let (pool, proxy, _scratch) = writer_pool(3, 0).await;
    let held = hold(&pool, 3).await;
    proxy.stall_established();
    let tasks: Vec<_> = held
        .into_iter()
        .map(|mut conn| {
            tokio::spawn(async move { sqlx::query("SELECT 1").execute(&mut *conn).await })
        })
        .collect();
    proxy.wait_for_held_sessions(3).await;
    for task in &tasks {
        task.abort();
    }
    for task in tasks {
        assert!(task.await.expect_err("query task aborted").is_cancelled());
    }
    await_capacity(&pool, 0).await;
    fresh_query(&pool).await;
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn aborted_writer_begin_reclaims_slot_and_allows_committed_write() {
    let (pool, proxy, scratch) = writer_pool(1, 0).await;
    let observer = scratch.connect().await;
    sqlx::query("CREATE TABLE release_probe (n int)")
        .execute(&observer)
        .await
        .unwrap();
    let mut conn = pool.acquire().await.unwrap();
    proxy.stall_established();
    let task = tokio::spawn(async move {
        let _tx = conn.begin().await.unwrap();
    });
    proxy.wait_for_held_sessions(1).await;
    task.abort();
    assert!(task.await.expect_err("BEGIN task aborted").is_cancelled());
    await_capacity(&pool, 0).await;
    sqlx::query("INSERT INTO release_probe VALUES (1)")
        .execute(&pool)
        .await
        .unwrap();
    let committed: i32 = sqlx::query_scalar("SELECT n FROM release_probe")
        .fetch_one(&observer)
        .await
        .expect("independent session sees commit");
    assert_eq!(committed, 1);
    pool.close().await;
    observer.close().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn transient_return_stall_reuses_healthy_writer_connection() {
    let (pool, proxy, _scratch) = writer_pool(1, 0).await;
    let mut conn = pool.acquire().await.unwrap();
    let before: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    proxy.set_paused(true);
    drop(conn);
    proxy.wait_for_held_sessions(1).await;
    proxy.resume_all();
    await_capacity(&pool, 1).await;
    let after: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        before, after,
        "a resumed return must reuse the healthy session"
    );
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn silent_return_replenishes_writer_minimum() {
    let (pool, proxy, _scratch) = writer_pool(1, 1).await;
    let held = hold(&pool, 1).await;
    let sessions = proxy.sessions();
    proxy.stall_established();
    drop(held);
    proxy.wait_for_held_sessions(1).await;
    await_capacity(&pool, 1).await;
    assert!(
        proxy.sessions() > sessions,
        "minimum must use a new physical session"
    );
    fresh_query(&pool).await;
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn silence_after_release_hook_does_not_strand_sqlx_ping() {
    let scratch = ScratchDb::create(&crate::test_support::database_url(), "release_hook").await;
    let options: sqlx::postgres::PgConnectOptions = scratch.url().parse().unwrap();
    let proxy = std::sync::Arc::new(
        PausingProxy::spawn(options.get_host().to_owned(), options.get_port()).await,
    );
    let hook_proxy = proxy.clone();
    let first = std::sync::atomic::AtomicBool::new(true);
    let pool = Db::read_pool_options(1)
        .min_connections(0)
        .acquire_timeout(TEST_LANE_ACQUIRE_TIMEOUT)
        .after_release(move |conn, _| {
            let proxy = hook_proxy.clone();
            let stall = first.swap(false, std::sync::atomic::Ordering::SeqCst);
            Box::pin(async move {
                let result = Db::reader_release_check(conn).await;
                assert!(matches!(result, Ok(true)), "release hook must succeed");
                if stall {
                    proxy.stall_established();
                }
                result
            })
        })
        .connect_with(options.host("127.0.0.1").port(proxy.port))
        .await
        .unwrap();
    drop(pool.acquire().await.unwrap());
    proxy.wait_for_held_sessions(1).await;
    await_capacity(&pool, 0).await;
    fresh_query(&pool).await;
    pool.close().await;
}
