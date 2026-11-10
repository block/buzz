//! Exact, bounded search-hit hydration without probing every events partition.

use buzz_core::{CommunityId, StoredEvent};
use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use sqlx::{PgConnection, QueryBuilder};

use crate::{Db, DbError, Result, RouteDecision, RoutePredicate};

const MAX_BATCH: usize = 500;

type Lookup = ([u8; 32], DateTime<Utc>);

fn validate(hits: &[([u8; 32], i64)]) -> Result<Vec<Lookup>> {
    if hits.len() > MAX_BATCH {
        return Err(DbError::InvalidData(
            "event hydration exceeds 500 hits".into(),
        ));
    }
    let mut keys = Vec::with_capacity(hits.len());
    for &(id, seconds) in hits {
        let timestamp = DateTime::from_timestamp(seconds, 0)
            .filter(|_| seconds >= 0)
            .ok_or(DbError::InvalidTimestamp(seconds))?;
        keys.push((id, timestamp));
    }
    // SQL IN semantics: duplicate inputs must not multiply hydrated rows.
    keys.sort_unstable();
    keys.dedup();
    Ok(keys)
}

fn lookup_sql(community: CommunityId, keys: &[Lookup]) -> QueryBuilder<sqlx::Postgres> {
    let mut query = QueryBuilder::new(
        "SELECT id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id \
         FROM events WHERE community_id = ",
    );
    query.push_bind(community.as_uuid());
    query.push(" AND deleted_at IS NULL AND (");
    for (index, (id, timestamp)) in keys.iter().enumerate() {
        if index != 0 {
            query.push(" OR ");
        }
        // Keep each id paired with its exact timestamp, not two independent IN
        // lists. Scalar partition-key predicates allow both plan-time pruning
        // and executor startup pruning when PostgreSQL chooses a generic plan.
        query.push("(created_at = ").push_bind(*timestamp);
        query.push(" AND id = ").push_bind(id.to_vec()).push(")");
    }
    query.push(")");
    query
}

async fn lookup_on(
    conn: &mut PgConnection,
    community: CommunityId,
    keys: &[Lookup],
) -> Result<Vec<StoredEvent>> {
    // Not a cached prepared statement: after five runs PostgreSQL may switch it
    // to a generic plan, which on PG16 prunes only at executor startup, after
    // every `events` partition and index is already locked. The SQL text also
    // varies with the batch size, which would fill the statement cache.
    let rows = lookup_sql(community, keys)
        .build()
        .persistent(false)
        .fetch_all(conn)
        .await?;
    let mut events = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(event) = crate::event::row_to_stored_event(row)? {
            events.push(event);
        }
    }
    Ok(events)
}

impl Db {
    /// Hydrate non-deleted search hits by paired `(id, created_at)` keys.
    ///
    /// Accepts at most 500 inputs (including duplicates), with nonnegative Unix
    /// seconds representable by chrono; invalid input errors before any checkout.
    /// Empty input returns immediately. Results are unordered and duplicates do
    /// not multiply rows; callers retain responsibility for search ordering and
    /// visibility. A stale, missing, deleted or wrong-community key is omitted.
    ///
    /// Routing is bounded-only: an id list carries no channel pin, so no fence
    /// floor can prove insert-completeness, and timestamps enable partition
    /// pruning, not a completeness proof. A missing row degrades to a skipped
    /// search hit downstream. The caller must use a classified search-hydration
    /// path (see [`Db::query_events_routed`] for the classification contract).
    #[datastore_span(name = "get_events_by_ids_at_routed", system = "postgresql")]
    pub async fn get_events_by_ids_at_routed(
        &self,
        path: &'static str,
        community: CommunityId,
        hits: &[([u8; 32], i64)],
    ) -> Result<Vec<StoredEvent>> {
        let keys = validate(hits)?;
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        match self
            .route_read(
                path,
                RoutePredicate::Bounded,
                crate::observability::ReaderOperation::SubscriptionHistory,
            )
            .await?
        {
            RouteDecision::Replica(mut tx, _entry, reason) => {
                match lookup_on(&mut tx, community, &keys).await {
                    Ok(events) => {
                        Self::record_route(path, "replica", reason);
                        return Ok(events);
                    }
                    Err(error) if error.is_statement_cancelled() => {
                        Self::record_route(path, "replica", "statement_cancelled");
                        return Err(error);
                    }
                    Err(error) => {
                        tracing::warn!(path, "replica read failed; re-running on writer: {error}");
                        Self::record_route(path, "writer", "replica_error");
                        // Release the reader and its history permit before the
                        // writer re-run, so a replica-error burst can't fill the cap.
                        drop(tx);
                    }
                }
            }
            RouteDecision::Writer => {}
        }
        let mut conn = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::SubscriptionHistory,
        )
        .await?;
        lookup_on(&mut conn, community, &keys).await
    }
}

#[cfg(test)]
mod postgres_tests;
