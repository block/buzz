//! App-owned follow-up work for a newly inserted `events` row.
//!
//! Two pieces of work follow every durable event insert:
//!
//! - **Push match enqueue.** Eligible event IDs are staged in memory. Commit
//!   submits them to a bounded best-effort worker using a dedicated connection.
//!   Push errors, delays, overload, and shutdown can drop wakes, never messages.
//! - **Channel TTL refresh.** A channel-scoped event pushes its ephemeral
//!   channel's `ttl_deadline` forward. [`AdmittedTx`] records the channel and
//!   runs the refresh as the last statement before COMMIT (see
//!   [`refresh_channel_ttls`]). A refresh failure is logged and counted, and
//!   the event still commits.
//!
//! This is the application-side replacement for the `events_enqueue_push_match`
//! and `events_refresh_channel_ttl` triggers (migrations 0023, 0024, 0040).
//! Updated writer connections suppress the surviving push trigger. Legacy
//! writers retain transactional enqueue until replaced. Every production
//! `INSERT INTO events` must call this module, enforced by source-policy tests.

use std::collections::BTreeSet;

use buzz_core::CommunityId;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::{AdmittedTx, DbError, Result};

/// Event kinds that wake push subscribers. Keep identical to the relay's
/// validated NIP-PL descriptor.
pub(crate) const PUSH_MATCH_KINDS: [i32; 4] = [9, 40002, 45001, 45003];

/// Kind 9007 creates the channel and initializes its deadline itself.
const KIND_CHANNEL_CREATE: i32 = 9007;

/// SQLSTATE `lock_not_available`, raised when `lock_timeout` expires.
const SQLSTATE_LOCK_NOT_AVAILABLE: &str = "55P03";

/// Follow-up for an event row inserted in an admitted transaction.
///
/// Call only when the INSERT affected a row, after releasing any savepoint
/// that may roll it back. Stages push work for after commit and records TTL work.
pub(crate) async fn after_admitted_insert(
    tx: &mut AdmittedTx,
    event_id: &[u8],
    kind: i32,
    channel_id: Option<Uuid>,
) -> Result<()> {
    tx.record_push_event(event_id, kind);
    tx.record_channel_event(channel_id, kind);
    Ok(())
}

/// The channel whose TTL an event of `kind` in `channel_id` refreshes, if any.
pub(crate) fn ttl_refresh_channel(channel_id: Option<Uuid>, kind: i32) -> Option<Uuid> {
    channel_id.filter(|_| kind != KIND_CHANNEL_CREATE)
}

/// Refresh the TTL deadline of every channel that received an event in this
/// transaction. [`AdmittedTx::commit`] calls this as its last work before
/// COMMIT, which keeps the deferred-trigger timing from migration 0024: the
/// deadline is computed at commit. An ephemeral channel's row lock, taken by
/// the UPDATE, is held through the remaining refreshes and the COMMIT round
/// trip; the trigger held it only inside COMMIT. A permanent channel's row is
/// never locked.
///
/// Channels are refreshed in sorted order, so two transactions that touch the
/// same channels cannot deadlock on the shared locks against an exclusive
/// waiter. Each refresh runs in its own savepoint. A failure is rolled back,
/// logged, and counted, and the event still commits, as with the trigger's
/// `EXCEPTION WHEN OTHERS`. A cancelled statement (`57014`) still rejects the
/// transaction, because plpgsql never caught it either.
pub(crate) async fn refresh_channel_ttls(
    conn: &mut PgConnection,
    community: CommunityId,
    channels: &BTreeSet<Uuid>,
) -> Result<()> {
    for &channel in channels {
        sqlx::query("SAVEPOINT channel_ttl_refresh")
            .execute(&mut *conn)
            .await?;
        match refresh_channel_ttl(conn, community, channel).await {
            Ok(()) => {
                sqlx::query("RELEASE SAVEPOINT channel_ttl_refresh")
                    .execute(&mut *conn)
                    .await?;
            }
            Err(error) => {
                // plpgsql `WHEN OTHERS` never caught `query_canceled`, so a
                // cancelled refresh has always rejected the event.
                if error.is_statement_cancelled() {
                    return Err(error);
                }
                sqlx::query("ROLLBACK TO SAVEPOINT channel_ttl_refresh")
                    .execute(&mut *conn)
                    .await?;
                sqlx::query("RELEASE SAVEPOINT channel_ttl_refresh")
                    .execute(&mut *conn)
                    .await?;
                let reason = if sqlstate(&error).as_deref() == Some(SQLSTATE_LOCK_NOT_AVAILABLE) {
                    "lock_timeout"
                } else {
                    "error"
                };
                metrics::counter!(
                    "buzz_db_channel_ttl_refresh_failures_total",
                    "reason" => reason,
                )
                .increment(1);
                tracing::warn!(
                    %error,
                    channel_id = %channel,
                    reason,
                    "channel TTL refresh failed; event committed without refreshing the deadline"
                );
            }
        }
    }
    Ok(())
}

async fn refresh_channel_ttl(
    conn: &mut PgConnection,
    community: CommunityId,
    channel: Uuid,
) -> Result<()> {
    // SHARED here, EXCLUSIVE in `update_channel` for TTL transitions: the same
    // total order the 0022 row lock gave, without serializing hot channels.
    sqlx::query("SELECT pg_advisory_xact_lock_shared(hashtextextended($1, 0))")
        .bind(crate::channel::channel_ttl_lock_key(community, channel))
        .execute(&mut *conn)
        .await?;
    // A separate statement, so its snapshot is taken after the lock is
    // granted. `ttl_seconds IS NOT NULL` filters a permanent channel before
    // any tuple lock is taken, so its row is never locked (migration 0024).
    sqlx::query(
        "UPDATE channels \
         SET ttl_deadline = clock_timestamp() + make_interval(secs => ttl_seconds) \
         WHERE community_id = $1 AND id = $2 \
           AND ttl_seconds IS NOT NULL \
           AND archived_at IS NULL \
           AND deleted_at IS NULL",
    )
    .bind(community.as_uuid())
    .bind(channel)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

fn sqlstate(error: &DbError) -> Option<String> {
    match error {
        DbError::Sqlx(sqlx::Error::Database(database_error)) => {
            database_error.code().map(|code| code.into_owned())
        }
        _ => None,
    }
}
