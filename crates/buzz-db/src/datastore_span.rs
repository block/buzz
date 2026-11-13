//! Runtime support for `#[datastore_span]`; not a public API.
//!
//! Every field here is written to the span exactly once, when the operation
//! finishes. A second `Span::record` of a field does not replace the first in
//! exported traces: tracing-opentelemetry appends another attribute with the
//! same key, so a span could claim both `writer` and `reader`. Route
//! decisions therefore go to a slot scoped to the innermost datastore span,
//! where a later decision (a writer re-run after a replica failure) replaces
//! an earlier one, and the slot is recorded once at the end.

use std::cell::Cell;
use std::future::Future;

use crate::error::DatastoreSpanError;
use crate::DbPoolRole;

/// The pool that served an operation, and the route that chose it. Opaque.
#[derive(Clone, Copy)]
pub struct Route {
    pool: DbPoolRole,
    decision: &'static str,
    reason: &'static str,
}

tokio::task_local! {
    static ROUTE: Cell<Option<Route>>;
}

/// Record that the innermost enclosing `#[datastore_span]` was served by
/// `pool` through `decision` for `reason`. Each value is a fixed label. A
/// later call within the same span replaces this one; outside any datastore
/// span it does nothing.
pub(crate) fn set_route(pool: DbPoolRole, decision: &'static str, reason: &'static str) {
    let _ = ROUTE.try_with(|slot| {
        slot.set(Some(Route {
            pool,
            decision,
            reason,
        }))
    });
}

/// Run one datastore operation with its own route slot, returning the route
/// it recorded, if any.
///
/// The operation is boxed before it is wrapped. Unboxed, every datastore
/// span would add the whole operation's state plus the task-local scope to
/// its caller's future, and debug builds of deeply nested operations
/// overflow a 2 MiB test-thread stack.
#[doc(hidden)]
pub fn scoped<F: Future>(operation: F) -> impl Future<Output = (F::Output, Option<Route>)> {
    let operation = Box::pin(operation);
    ROUTE.scope(Cell::new(None), async move {
        let output = operation.await;
        let route = ROUTE.with(Cell::get);
        (output, route)
    })
}

/// Record `db.pool.role`, and for a routed operation `db.route.decision`
/// and `db.route.reason`, on `span`. `default_pool` is the macro's `pool`
/// argument, used when no route was recorded.
#[doc(hidden)]
pub fn record_route(span: &tracing::Span, default_pool: &'static str, route: Option<Route>) {
    match route {
        Some(route) => {
            span.record("db.pool.role", route.pool.as_str());
            span.record("db.route.decision", route.decision);
            span.record("db.route.reason", route.reason);
        }
        None => {
            span.record("db.pool.role", default_pool);
        }
    }
}

/// Record `error`'s fixed classification on `span`: `error.type`, and for a
/// Postgres error `db.response.status_code` (SQLSTATE) and
/// `db.error.constraint`.
///
/// Both driver fields are checked against their expected shape, so a value
/// that is not a SQLSTATE or a plain identifier is dropped rather than
/// recorded.
#[doc(hidden)]
pub fn record_error<E: DatastoreSpanError + ?Sized>(span: &tracing::Span, error: &E) {
    span.record("error.type", error.span_error_type());
    let Some(sqlx::Error::Database(db)) = error.span_sqlx_error() else {
        return;
    };
    if let Some(code) = db.code().filter(|code| is_sqlstate(code)) {
        span.record("db.response.status_code", code.as_ref());
    }
    if let Some(constraint) = db.constraint().filter(|name| is_plain_identifier(name)) {
        span.record("db.error.constraint", constraint);
    }
}

/// A SQLSTATE is five characters from `[0-9A-Z]`.
fn is_sqlstate(code: &str) -> bool {
    code.len() == 5
        && code
            .bytes()
            .all(|b| b.is_ascii_digit() || b.is_ascii_uppercase())
}

/// An unquoted Postgres identifier of at most 63 bytes. Buzz names its
/// constraints this way; anything else is not a schema name we chose.
fn is_plain_identifier(name: &str) -> bool {
    (1..=63).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}
