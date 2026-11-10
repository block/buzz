//! Writer-pool connection hygiene against a real Postgres.

use super::*;

/// A `BEGIN` cancelled before Postgres answers must not hand its pooled
/// connection to the next borrower still inside that transaction.
///
/// sqlx 0.9.0 counted a transaction only once `BEGIN` was acknowledged, so a
/// begin dropped in flight queued no `ROLLBACK` and the release ping consumed
/// the late reply (fixed upstream in transact-rs/sqlx#4394). The writer pool
/// has no release check, so the next borrower's autocommit statements ran
/// inside the leftover transaction: `25001` on its next `BEGIN`, `25P02` on
/// every statement once one of them failed, and its writes were left
/// uncommitted. Covers the event read's custom `BEGIN ... READ ONLY`
/// ([`crate::event::fetch_with_read_deadline`]) and a plain `begin()`.
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires Postgres"]
async fn begin_cancelled_in_flight_leaves_no_writer_transaction_open() {
    let (seed, _scratch) = ScratchDb::migrated(&admin_url().await, "begin_cxl").await;
    let options = seed.connect_options();
    let proxy =
        crate::test_support::PausingProxy::spawn(options.get_host().to_owned(), options.get_port())
            .await;
    // One connection, so every checkout is the one the cancelled begin used;
    // like the writer pool, no release check.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_lazy_with((*options).clone().host("127.0.0.1").port(proxy.port));
    sqlx::query("CREATE TABLE begin_cancel_probe (n int NOT NULL)")
        .execute(&pool)
        .await
        .expect("create probe table");

    for (n, form) in [(1, "event read BEGIN ... READ ONLY"), (2, "plain begin()")] {
        let mut conn = pool.acquire().await.expect("checkout before cancel");
        proxy.set_paused(true);
        let budget = Duration::from_millis(200);
        let cancelled = if n == 1 {
            tokio::time::timeout(
                budget,
                crate::event::fetch_with_read_deadline(
                    &mut conn,
                    sqlx::query("SELECT 1"),
                    crate::event::ReadDeadline::PerRead,
                ),
            )
            .await
            .is_err()
        } else {
            tokio::time::timeout(budget, conn.begin()).await.is_err()
        };
        assert!(
            cancelled,
            "{form}: the paused proxy must hold BEGIN in flight"
        );
        drop(conn);
        proxy.set_paused(false);

        let mut next = pool.acquire().await.expect("checkout after cancel");
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *next)
            .await
            .expect("next borrower's backend pid");
        // A leftover `READ ONLY` transaction fails this write; a read-write one
        // leaves it uncommitted (checked below).
        if let Err(e) = sqlx::query("INSERT INTO begin_cancel_probe (n) VALUES ($1)")
            .bind(n)
            .execute(&mut *next)
            .await
        {
            panic!("{form}: the next borrower's autocommit write failed: {e}");
        }
        let state: Option<String> =
            sqlx::query_scalar("SELECT state FROM pg_stat_activity WHERE pid = $1")
                .bind(pid)
                .fetch_one(&seed)
                .await
                .expect("read the next borrower's backend state");
        assert_eq!(
            state.as_deref(),
            Some("idle"),
            "{form}: the next borrower runs inside the cancelled BEGIN's transaction"
        );
    }

    // Read from a connection outside the proxied pool: an autocommit write
    // that ran inside a leftover transaction is not committed.
    let committed: Vec<i32> = sqlx::query_scalar("SELECT n FROM begin_cancel_probe ORDER BY n")
        .fetch_all(&seed)
        .await
        .expect("read probe rows");
    assert_eq!(committed, vec![1, 2], "every autocommit write must commit");
    pool.close().await;
}
