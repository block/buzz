//! Cancel a search statement server-side when its caller goes away.
//!
//! Dropping a sqlx query future does not stop its statement: Postgres runs it
//! until it next writes to the socket, and a ranked `LIMIT` query writes
//! nothing until it has matched and ranked every candidate row. Typeahead
//! abandons a search on almost every keystroke (the client aborts its
//! request and the relay drops the handler), so without this each abandoned
//! search would still run to the end.
//!
//! sqlx does not expose a connection's cancel key, so [`fetch_all`] reads the
//! backend's `(pid, backend_start)` before the statement. If the caller drops
//! the future first, a spawned task signals that backend with
//! `pg_cancel_backend` from another pool connection. The pair names one
//! backend for its lifetime: a reader endpoint can route the cancelling
//! connection to a different replica, where the pair matches no backend, so a
//! cancel never reaches an unrelated statement; the task tries another
//! connection instead. The abandoned connection stays checked out until the
//! task is done, so its backend cannot start another caller's statement
//! before the signal lands. A cancel that cannot reach the backend leaves the
//! statement to the pool's statement timeout ([`crate::pool_options`]).

use std::time::Duration;

use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgArguments, PgRow};
use sqlx::query::Query;
use sqlx::{PgPool, Postgres};

/// Pool connections a cancel tries before it gives up on reaching the
/// backend. Each miss is a connection to another replica.
const CANCEL_ATTEMPTS: usize = 3;

/// Bound on each cancel attempt's pool checkout; a saturated pool gives up
/// rather than queueing behind the searches it is trying to stop.
const CANCEL_ACQUIRE_TIMEOUT: Duration = Duration::from_millis(500);

/// `backend_start` as integer microseconds, so it round-trips exactly.
macro_rules! backend_start_us {
    () => {
        "(EXTRACT(EPOCH FROM backend_start) * 1000000)::bigint"
    };
}

/// This connection's backend: `(pid, backend_start in microseconds)`.
const BACKEND_SQL: &str = concat!(
    "SELECT pid, ",
    backend_start_us!(),
    " FROM pg_stat_activity WHERE pid = pg_backend_pid()"
);

/// Signal the backend `($1 pid, $2 backend_start in microseconds)`: one row
/// when it is on this server, none otherwise. Public only for its tests.
#[doc(hidden)]
pub const CANCEL_SQL: &str = concat!(
    "SELECT pg_cancel_backend(pid) FROM pg_stat_activity WHERE pid = $1 AND ",
    backend_start_us!(),
    " = $2"
);

/// Run `query` on `conn`, a connection from `pool`, cancelling its statement
/// on the server if the returned future is dropped before it completes.
/// Callers pass the connection so earlier steps of the same search reuse one
/// pool checkout.
pub(crate) async fn fetch_all(
    pool: &PgPool,
    mut conn: PoolConnection<Postgres>,
    query: Query<'_, Postgres, PgArguments>,
) -> Result<Vec<PgRow>, sqlx::Error> {
    let backend: (i32, i64) = sqlx::query_as(BACKEND_SQL).fetch_one(&mut *conn).await?;
    let mut armed = CancelOnDrop {
        pool: pool.clone(),
        backend,
        conn: Some(conn),
    };
    // `conn` is `Some` until this function or `Drop` takes it.
    let Some(conn) = armed.conn.as_deref_mut() else {
        return Err(sqlx::Error::PoolClosed);
    };
    let rows = query.fetch_all(conn).await;
    // Finished (rows or error): nothing left to cancel, and the connection
    // goes back to the pool.
    armed.conn = None;
    rows
}

/// The connection of a statement still in flight, and how to signal its
/// backend. Dropping it with the connection still held cancels the statement.
struct CancelOnDrop {
    pool: PgPool,
    backend: (i32, i64),
    conn: Option<PoolConnection<Postgres>>,
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        let Some(conn) = self.conn.take() else {
            return;
        };
        // No runtime (process shutdown): the session ends with the process.
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let pool = self.pool.clone();
        let backend = self.backend;
        runtime.spawn(async move {
            let outcome = cancel(&pool, backend).await;
            metrics::counter!("buzz_search_cancel_total", "outcome" => outcome).increment(1);
            // Only now may the backend take another statement.
            drop(conn);
        });
    }
}

/// Signal `backend` from another connection of `pool`; returns the outcome
/// label for `buzz_search_cancel_total`.
async fn cancel(pool: &PgPool, (pid, started_us): (i32, i64)) -> &'static str {
    // Connections that reached another replica stay checked out, so each
    // attempt gets a different connection.
    let mut elsewhere = Vec::with_capacity(CANCEL_ATTEMPTS);
    for _ in 0..CANCEL_ATTEMPTS {
        let mut conn = match tokio::time::timeout(CANCEL_ACQUIRE_TIMEOUT, pool.acquire()).await {
            Ok(Ok(conn)) => conn,
            Ok(Err(e)) => {
                tracing::warn!(error_kind = %buzz_db::error::sqlx_error_kind(&e), pid, "search cancel could not get a connection");
                return "error";
            }
            Err(_) => return "pool_busy",
        };
        match sqlx::query_scalar::<_, bool>(CANCEL_SQL)
            .bind(pid)
            .bind(started_us)
            .fetch_optional(&mut *conn)
            .await
        {
            Ok(None) => elsewhere.push(conn),
            Ok(Some(signalled)) => return signal_outcome(signalled),
            Err(e) => {
                tracing::warn!(error_kind = %buzz_db::error::sqlx_error_kind(&e), pid, "search cancel failed");
                return "error";
            }
        }
    }
    "missed"
}

/// Outcome label for a [`CANCEL_SQL`] row: `pg_cancel_backend` returns false
/// when the signal could not be sent (the backend exited in between).
fn signal_outcome(signalled: bool) -> &'static str {
    if signalled {
        "cancelled"
    } else {
        "signal_failed"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_sent_signal_counts_as_cancelled() {
        assert_eq!(signal_outcome(true), "cancelled");
        assert_eq!(signal_outcome(false), "signal_failed");
    }
}
