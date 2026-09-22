//! NIP-CW thread-mode newest-first reply windows. This path deliberately does not change
//! legacy forward threads or the permissive generic event reconstruction path.

use buzz_core::{
    thread_window::{Cursor, Request, MAX_LIMIT, ROW_KINDS},
    CommunityId, StoredEvent,
};
use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool, QueryBuilder, Row};
use uuid::Uuid;

use crate::{
    event::row_to_stored_event, Db, DbError, ReadSession, ReadSessionInner, Result, RouteDecision,
    RoutePredicate,
};
use buzz_datastore_tracing::datastore_span;

/// One bounded raw scan; damaged reply rows can shorten `rows`, never the
/// authoritative scan bounds.
#[derive(Debug)]
pub struct ThreadWindow {
    /// Reconstructed replies, newest first (id ascending for ties).
    pub rows: Vec<StoredEvent>,
    /// Another eligible raw reply exists after this page.
    pub has_more: bool,
    /// Last retained raw candidate, or None iff exhausted.
    pub next_cursor: Option<Cursor>,
    /// Root exists in this channel, possibly as a tombstone. Only then may
    /// the bridge expand root auxiliary events.
    pub root_in_channel: bool,
}

fn invalid(message: impl Into<String>) -> DbError {
    DbError::InvalidData(message.into())
}

fn cursor_key(cursor: &Cursor) -> Result<(DateTime<Utc>, Vec<u8>)> {
    let ts = cursor.timestamp().map_err(invalid)?;
    let id = hex::decode(&cursor.id).map_err(|_| invalid("invalid thread cursor id"))?;
    if id.len() != 32 {
        return Err(invalid("invalid thread cursor length"));
    }
    Ok((ts, id))
}

fn scan_cursor(row: &sqlx::postgres::PgRow) -> Result<Cursor> {
    let created_at: DateTime<Utc> = row.try_get("created_at")?;
    let id: Vec<u8> = row.try_get("id")?;
    if id.len() != 32 || created_at.timestamp() < 0 {
        return Err(invalid("unrepresentable thread scan position"));
    }
    Ok(Cursor {
        created_at: created_at.timestamp(),
        id: hex::encode(id),
    })
}

/// Every new-mode SQL statement has a server-side deadline as well as the
/// bridge's whole-operation deadline. SET LOCAL cannot leak to pooled callers.
async fn set_deadline(conn: &mut PgConnection) -> Result<()> {
    sqlx::query("SET LOCAL statement_timeout = '4000ms'")
        .execute(&mut *conn)
        .await?;
    sqlx::query("SET LOCAL lock_timeout = '1000ms'")
        .execute(&mut *conn)
        .await?;
    Ok(())
}

async fn select_window(
    conn: &mut PgConnection,
    community: CommunityId,
    request: &Request,
) -> Result<ThreadWindow> {
    if !(1..=MAX_LIMIT).contains(&request.limit)
        || !(1..=100).contains(&request.depth)
        || request.kinds.is_empty()
        || request.kinds.iter().any(|k| !ROW_KINDS.contains(k))
    {
        return Err(invalid("invalid thread window arguments"));
    }
    let root = hex::decode(&request.root).map_err(|_| invalid("invalid thread root"))?;
    if root.len() != 32 {
        return Err(invalid("invalid thread root length"));
    }
    // Retain tombstones: deleting a root must not make its readable replies
    // disappear. Never follow a root belonging to another channel/community.
    let root_in_channel: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM events WHERE community_id = $1 AND channel_id = $2 AND id = $3 AND kind = ANY($4))")
        .bind(community.as_uuid()).bind(request.channel).bind(&root)
        .bind(ROW_KINDS.map(|k| k as i32).to_vec())
        .fetch_one(&mut *conn).await?;
    if !root_in_channel {
        return Ok(ThreadWindow {
            rows: vec![],
            has_more: false,
            next_cursor: None,
            root_in_channel,
        });
    }
    // Order metadata before event lookups. With stale statistics a root-wide
    // sort can otherwise execute one lateral lookup for every reply first.
    // OFFSET 0 keeps this boundary without limiting candidates prematurely.
    let mut q = QueryBuilder::new(
        "SELECT e.id, e.pubkey, e.created_at, e.kind, e.tags, e.content, e.sig, e.received_at, e.channel_id \
         FROM (SELECT tm.community_id, tm.event_id, tm.event_created_at \
         FROM thread_metadata tm WHERE tm.community_id = ",
    );
    q.push_bind(community.as_uuid())
        .push(" AND tm.root_event_id = ")
        .push_bind(root)
        .push(" AND tm.channel_id = ")
        .push_bind(request.channel)
        .push(" AND tm.depth BETWEEN 1 AND ")
        .push_bind(request.depth as i32);
    if let Some(cursor) = &request.cursor {
        let (ts, id) = cursor_key(cursor)?;
        // The redundant upper timestamp bound gives PostgreSQL an index range
        // start instead of filtering all newer keys at a deep cursor.
        q.push(" AND tm.event_created_at <= ")
            .push_bind(ts)
            .push(" AND (tm.event_created_at < ")
            .push_bind(ts)
            .push(" OR (tm.event_created_at = ")
            .push_bind(ts)
            .push(" AND tm.event_id > ")
            .push_bind(id)
            .push("))");
    }
    // Fetch by the event PK before testing visibility, so missing statistics
    // cannot turn a selective live/kind index into a root-wide scan per reply.
    // PK uniqueness makes the inner LIMIT 1 exact. All eligibility predicates
    // still precede the *outer* limit+1 that establishes page bounds.
    q.push(" ORDER BY tm.event_created_at DESC, tm.event_id ASC OFFSET 0) tm \
        JOIN LATERAL (SELECT e.id, e.pubkey, e.created_at, e.kind, e.tags, e.content, e.sig, e.received_at, e.channel_id, e.deleted_at \
        FROM events e WHERE e.community_id = tm.community_id \
        AND e.created_at = tm.event_created_at AND e.id = tm.event_id LIMIT 1) e ON true \
        WHERE e.channel_id = ")
        .push_bind(request.channel)
        .push(" AND e.deleted_at IS NULL AND e.kind = ANY(")
        .push_bind(request.kinds.iter().map(|k| *k as i32).collect::<Vec<_>>())
        .push(") ORDER BY tm.event_created_at DESC, tm.event_id ASC LIMIT ")
        .push_bind(i64::from(request.limit) + 1);
    // Selectivity varies drastically with root/depth/cursor. A cached generic
    // plan can sort the entire root (and exceeded the SQL deadline in paging
    // tests). An unnamed statement keeps parameter-aware planning for this
    // bounded query without changing pooled session or legacy settings.
    let mut raw = q.build().persistent(false).fetch_all(&mut *conn).await?;
    let has_more = raw.len() > request.limit as usize;
    raw.truncate(request.limit as usize);
    let next_cursor = if has_more {
        raw.last().map(scan_cursor).transpose()?
    } else {
        None
    };
    let mut rows = Vec::with_capacity(raw.len());
    for row in raw {
        if let Some(event) = row_to_stored_event(row)? {
            rows.push(event);
        }
    }
    Ok(ThreadWindow {
        rows,
        has_more,
        next_cursor,
        root_in_channel,
    })
}

async fn writer_window(
    pool: &PgPool,
    community: CommunityId,
    request: &Request,
) -> Result<ThreadWindow> {
    let conn = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::SubscriptionHistory,
    )
    .await?;
    let mut tx = sqlx::Transaction::begin(conn, None).await?;
    set_deadline(&mut tx).await?;
    let window = select_window(&mut tx, community, request).await?;
    tx.rollback().await?;
    Ok(window)
}

impl Db {
    /// Newest-first thread page and its proved replica transaction. Backward
    /// cursors supply an upper bound, including terminal pages; no forward
    /// thread "last delivered row is newest" inference is used. Head routing
    /// inherits the existing default-off budget. Writer follow-ups are pooled,
    /// not a snapshot spanning the response or subsequent history pages.
    #[datastore_span(name = "get_thread_window", system = "postgresql")]
    pub async fn get_thread_window_with_session(
        &self,
        community: CommunityId,
        request: &Request,
    ) -> Result<(ThreadWindow, ReadSession)> {
        let cursor = request.cursor.as_ref().map(cursor_key).transpose()?;
        let path = if cursor.is_some() {
            "thread_window_cursor"
        } else {
            "thread_window_head"
        };
        if let RouteDecision::Replica(mut tx, _, reason) = self
            .route_read(
                path,
                RoutePredicate::from_channel_cursor(request.channel, &cursor),
                crate::observability::ReaderOperation::SubscriptionHistory,
            )
            .await
        {
            let result = async {
                set_deadline(&mut tx).await?;
                select_window(&mut tx, community, request).await
            }
            .await;
            match result {
                Ok(window) if !window.root_in_channel => {
                    // The cursor covers reply insertions, not a root authored
                    // later than its replies. Recheck missing anchors on writer.
                    Self::record_route(path, "writer", "missing_root");
                }
                Ok(window) => {
                    Self::record_route(path, "replica", reason);
                    return Ok((
                        window,
                        ReadSession {
                            inner: ReadSessionInner::Replica {
                                tx,
                                writer: self.pool.clone(),
                            },
                        },
                    ));
                }
                Err(error @ DbError::InvalidData(_)) => return Err(error),
                Err(error) => {
                    tracing::warn!(%error, path, "replica thread window failed; re-running on writer");
                    Self::record_route(path, "writer", "replica_error");
                }
            }
        }
        let window = writer_window(&self.pool, community, request).await?;
        Ok((
            window,
            ReadSession {
                inner: ReadSessionInner::Writer(self.pool.clone()),
            },
        ))
    }
}

/// Strict auxiliary scan. Targets are chunked by the bridge, limiting SQL
/// expression size. Access is applied before the probe, including channel-less
/// deletions. Deleted aux payloads are NOT returned, but their IDs are retained
/// to discover deletions-of-aux on the second hop.
pub struct AuxQuery<'a> {
    /// Host-bound community.
    pub community: CommunityId,
    /// At most 200 retained target IDs (never the reply sentinel).
    pub targets: &'a [String],
    /// Auxiliary kinds for this hop.
    pub kinds: &'a [u32],
    /// Fresh writer-authorized channels for this page.
    pub accessible: &'a [Uuid],
    /// Last raw auxiliary scan position, not last delivered event.
    pub cursor: Option<Cursor>,
}

/// Fixed raw auxiliary page budget (the probe is one extra row).
pub const AUX_LIMIT: usize = 1000;

/// Aggregate auxiliary work allowance for one HTTP query, not one window.
/// Passed through replica retries so degradation cannot reset the allowance.
#[derive(Default)]
pub struct AuxBudget {
    queries: usize,
    rows: usize,
}

impl AuxBudget {
    fn query(&mut self) -> Result<()> {
        self.queries += 1;
        if self.queries > 64 {
            return Err(invalid("thread auxiliary closure exceeds query budget"));
        }
        Ok(())
    }

    fn rows(&mut self, count: usize) -> Result<()> {
        self.rows = self.rows.saturating_add(count);
        if self.rows > 8192 {
            return Err(invalid("thread auxiliary closure exceeds raw row budget"));
        }
        Ok(())
    }
}

/// A strict auxiliary page with raw traversal metadata.
pub struct AuxPage {
    /// Live events. Unreconstructable live auxiliary events cause an error.
    pub events: Vec<StoredEvent>,
    /// Raw IDs including tombstones, for deletion-of-aux discovery.
    pub target_ids: Vec<String>,
    /// Last retained scan position when another raw candidate exists.
    pub next_cursor: Option<Cursor>,
}

async fn select_aux(
    conn: &mut PgConnection,
    query: &AuxQuery<'_>,
    budget: &mut AuxBudget,
) -> Result<AuxPage> {
    if query.targets.is_empty() || query.targets.len() > 200 {
        return Err(invalid("invalid thread auxiliary target batch"));
    }
    budget.query()?;
    let mut q = QueryBuilder::new(
        "SELECT id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id, deleted_at \
         FROM events WHERE community_id = ");
    q.push_bind(query.community.as_uuid())
        .push(" AND ((channel_id IS NULL AND kind IN (5, 9005)) OR channel_id = ANY(")
        .push_bind(query.accessible)
        .push("))")
        .push(" AND kind = ANY(")
        .push_bind(query.kinds.iter().map(|k| *k as i32).collect::<Vec<_>>())
        .push(") AND (");
    for (index, target) in query.targets.iter().enumerate() {
        if index != 0 {
            q.push(" OR ");
        }
        q.push("tags @> ")
            .push_bind(serde_json::json!([["e", target]]));
    }
    q.push(")");
    if let Some(cursor) = &query.cursor {
        let (ts, id) = cursor_key(cursor)?;
        q.push(" AND (created_at < ")
            .push_bind(ts)
            .push(" OR (created_at = ")
            .push_bind(ts)
            .push(" AND id > ")
            .push_bind(id)
            .push("))");
    }
    q.push(" ORDER BY created_at DESC, id ASC LIMIT ")
        .push_bind(AUX_LIMIT as i64 + 1);
    let mut raw = q.build().fetch_all(&mut *conn).await?;
    budget.rows(raw.len())?;
    let has_more = raw.len() > AUX_LIMIT;
    raw.truncate(AUX_LIMIT);
    let next_cursor = if has_more {
        raw.last().map(scan_cursor).transpose()?
    } else {
        None
    };
    let mut events = Vec::with_capacity(raw.len());
    let mut target_ids = Vec::with_capacity(raw.len());
    for row in raw {
        // Validate IDs even for deleted payloads: no ambiguous continuation.
        target_ids.push(scan_cursor(&row)?.id);
        if row
            .try_get::<Option<DateTime<Utc>>, _>("deleted_at")?
            .is_none()
        {
            events
                .push(row_to_stored_event(row)?.ok_or_else(|| {
                    invalid("cannot reconstruct required thread auxiliary event")
                })?);
        }
    }
    Ok(AuxPage {
        events,
        target_ids,
        next_cursor,
    })
}

async fn writer_aux(
    pool: &PgPool,
    query: &AuxQuery<'_>,
    budget: &mut AuxBudget,
) -> Result<AuxPage> {
    let conn = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::SubscriptionHistory,
    )
    .await?;
    let mut tx = sqlx::Transaction::begin(conn, None).await?;
    set_deadline(&mut tx).await?;
    let page = select_aux(&mut tx, query, budget).await?;
    tx.rollback().await?;
    Ok(page)
}

impl ReadSession {
    /// Strict NIP-CW thread-mode auxiliary scan on the same proved snapshot, with the
    /// existing permanent writer degradation on mid-request replica failure.
    #[datastore_span(name = "thread_window_aux", system = "postgresql")]
    pub async fn thread_window_aux(
        &mut self,
        query: &AuxQuery<'_>,
        budget: &mut AuxBudget,
    ) -> Result<AuxPage> {
        let writer = match &mut self.inner {
            ReadSessionInner::Replica { tx, writer } => match select_aux(tx, query, budget).await {
                Ok(page) => return Ok(page),
                Err(error @ DbError::InvalidData(_)) => return Err(error),
                Err(error) => {
                    tracing::warn!(%error, "thread auxiliary read failed; degrading to writer");
                    metrics::counter!("buzz_db_read_session_degraded").increment(1);
                    writer.clone()
                }
            },
            ReadSessionInner::Writer(pool) => return writer_aux(pool, query, budget).await,
        };
        self.inner = ReadSessionInner::Writer(writer.clone());
        writer_aux(&writer, query, budget).await
    }
}

#[cfg(test)]
mod postgres_tests;
