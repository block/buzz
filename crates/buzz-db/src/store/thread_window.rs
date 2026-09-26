//! NIP-CW thread-mode newest-first reply windows. This path deliberately does not change
//! legacy forward threads or the permissive generic event reconstruction path.

use buzz_core::{
    thread_window::{Cursor, Request, MAX_LIMIT, ROW_KINDS},
    CommunityId, StoredEvent,
};
use chrono::{DateTime, Utc};
use futures_util::TryStreamExt;
use sqlx::{PgConnection, PgPool, QueryBuilder, Row};
use uuid::Uuid;

use crate::{
    event::{push_e_tag_filter, row_to_stored_event},
    Db, DbError, ReadSession, ReadSessionInner, Result, RouteDecision, RoutePredicate,
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
    /// A supported conversation root exists in this channel, possibly as a
    /// tombstone. Only then may the bridge serve rows, auxiliary events or bounds.
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
    budget: &mut ScanBudget,
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
        "SELECT e.id, e.pubkey, e.created_at, e.kind, e.tags, e.content, e.sig, e.received_at, e.channel_id, \
         octet_length(e.content) + octet_length(e.tags::text) AS payload_bytes \
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
    // Charge every consumed payload before reconstruction, including damaged
    // rows and the limit+1 probe. The caller owns this ledger across all
    // windows, auxiliary scans and replica-to-writer retries.
    let mut raw = q.build().persistent(false).fetch(&mut *conn);
    let mut rows = Vec::new();
    let mut retained = 0;
    let mut last_cursor = None;
    let mut has_more = false;
    while let Some(row) = raw.try_next().await? {
        budget.rows(1)?;
        let payload_bytes: i32 = row.try_get("payload_bytes")?;
        budget.bytes(payload_bytes as usize)?;
        if retained == request.limit {
            has_more = true;
            break;
        }
        last_cursor = Some(scan_cursor(&row)?);
        retained += 1;
        if let Some(event) = row_to_stored_event(row)? {
            rows.push(event);
        }
    }
    let next_cursor = if has_more { last_cursor } else { None };
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
    budget: &mut ScanBudget,
) -> Result<ThreadWindow> {
    let conn = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::SubscriptionHistory,
    )
    .await?;
    let mut tx = sqlx::Transaction::begin(conn, None).await?;
    set_deadline(&mut tx).await?;
    let window = select_window(&mut tx, community, request, budget).await?;
    tx.rollback().await?;
    Ok(window)
}

impl Db {
    /// Newest-first thread page and its proved replica transaction. Backward
    /// cursors supply an upper bound, including terminal pages; no forward
    /// thread "last delivered row is newest" inference is used. Head routing
    /// inherits the existing default-off budget. Writer follow-ups are pooled,
    /// not a snapshot spanning the response or subsequent history pages. The
    /// caller must share `budget` with every window and auxiliary scan in the request.
    #[datastore_span(name = "get_thread_window", system = "postgresql")]
    pub async fn get_thread_window_with_session(
        &self,
        community: CommunityId,
        request: &Request,
        budget: &mut ScanBudget,
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
                select_window(&mut tx, community, request, budget).await
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
                Err(error @ (DbError::InvalidData(_) | DbError::ThreadWindowBudgetExceeded(_))) => {
                    return Err(error);
                }
                Err(error) => {
                    tracing::warn!(%error, path, "replica thread window failed; re-running on writer");
                    Self::record_route(path, "writer", "replica_error");
                }
            }
        }
        let window = writer_window(&self.pool, community, request, budget).await?;
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

/// Aggregate reply and auxiliary scan allowance for one query, not one window.
/// Bytes include content and tags; the bridge separately bounds serialized output.
/// Passed through replica retries so degradation cannot reset the allowance.
#[derive(Default)]
pub struct ScanBudget {
    queries: usize,
    rows: usize,
    bytes: usize,
}

impl ScanBudget {
    fn query(&mut self) -> Result<()> {
        self.queries += 1;
        if self.queries > 64 {
            return Err(DbError::ThreadWindowBudgetExceeded("query"));
        }
        Ok(())
    }

    fn bytes(&mut self, count: usize) -> Result<()> {
        self.bytes = self.bytes.saturating_add(count);
        if self.bytes > 8 * 1024 * 1024 {
            return Err(DbError::ThreadWindowBudgetExceeded("payload byte"));
        }
        Ok(())
    }

    fn rows(&mut self, count: usize) -> Result<()> {
        self.rows = self.rows.saturating_add(count);
        if self.rows > 8192 {
            return Err(DbError::ThreadWindowBudgetExceeded("raw row"));
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

/// SQL for [`select_aux`]. The target set is one array-bound containment
/// filtered inside a MATERIALIZED CTE, with ORDER BY/LIMIT outside: in the
/// same scope the planner walks the pkey backward over the whole community.
/// The fence materializes all matching rows before the outer sort/limit. The
/// 200-target cap and 1,000-row page do not bound match cardinality; the
/// statement deadline limits execution time, while `ScanBudget` limits the
/// rows/bytes the application consumes.
fn build_aux_sql(query: &AuxQuery<'_>) -> Result<QueryBuilder<sqlx::Postgres>> {
    let mut q = QueryBuilder::new(
        "WITH m AS MATERIALIZED (SELECT id, pubkey, created_at, kind, tags, content, sig, \
         received_at, channel_id, deleted_at, \
         octet_length(content) + octet_length(tags::text) AS payload_bytes \
         FROM events WHERE community_id = ",
    );
    q.push_bind(query.community.as_uuid())
        .push(" AND ((channel_id IS NULL AND kind IN (5, 9005)) OR channel_id = ANY(")
        .push_bind(query.accessible)
        .push("))")
        .push(" AND kind = ANY(")
        .push_bind(query.kinds.iter().map(|k| *k as i32).collect::<Vec<_>>())
        .push(")");
    // JSONB containment is an indexable prefilter, not a positional tag match.
    push_e_tag_filter(&mut q, "", query.targets);
    q.push(
        " AND EXISTS (SELECT 1 FROM jsonb_array_elements(tags) tag \
        WHERE tag->>0 = 'e' AND tag->>1 = ANY(",
    )
    .push_bind(query.targets)
    .push("))");
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
    q.push(") SELECT * FROM m ORDER BY created_at DESC, id ASC LIMIT ")
        .push_bind(AUX_LIMIT as i64 + 1);
    Ok(q)
}

async fn select_aux(
    conn: &mut PgConnection,
    query: &AuxQuery<'_>,
    budget: &mut ScanBudget,
) -> Result<AuxPage> {
    if query.targets.is_empty() || query.targets.len() > 200 {
        return Err(invalid("invalid thread auxiliary target batch"));
    }
    budget.query()?;
    let mut q = build_aux_sql(query)?;
    // Never collect a full raw page: 1,001 ingest-valid edits can contain
    // 250 MiB. Charge each row (including tombstones and the probe) before
    // reconstruction, and preserve this request-wide ledger across retries.
    let mut raw = q.build().fetch(&mut *conn);
    let mut events = Vec::new();
    let mut target_ids = Vec::new();
    let mut last_cursor = None;
    let mut next_cursor = None;
    while let Some(row) = raw.try_next().await? {
        budget.rows(1)?;
        let payload_bytes: i32 = row.try_get("payload_bytes")?;
        budget.bytes(payload_bytes as usize)?;
        if target_ids.len() == AUX_LIMIT {
            next_cursor = last_cursor;
            break;
        }
        // Validate IDs even for deleted payloads: no ambiguous continuation.
        let cursor = scan_cursor(&row)?;
        target_ids.push(cursor.id.clone());
        last_cursor = Some(cursor);
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
    budget: &mut ScanBudget,
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
        budget: &mut ScanBudget,
    ) -> Result<AuxPage> {
        let writer = match &mut self.inner {
            ReadSessionInner::Replica { tx, writer } => match select_aux(tx, query, budget).await {
                Ok(page) => return Ok(page),
                Err(error @ (DbError::InvalidData(_) | DbError::ThreadWindowBudgetExceeded(_))) => {
                    return Err(error);
                }
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

#[cfg(test)]
mod aux_sql_shape_tests {
    use super::*;

    /// The reply-closure target set must be one array-bound containment inside
    /// the fence, never an N-way `OR`, with ORDER BY/LIMIT outside it.
    #[test]
    fn aux_query_filters_in_materialized_fence_before_order_limit() {
        let targets: Vec<String> = (0..200).map(|i| format!("{i:064x}")).collect();
        let accessible = [Uuid::nil()];
        let query = AuxQuery {
            community: CommunityId::from_uuid(Uuid::nil()),
            targets: &targets,
            kinds: &[7],
            accessible: &accessible,
            cursor: None,
        };
        let qb = build_aux_sql(&query).expect("valid aux query");
        let sql = qb.sql().as_str().to_owned();

        assert!(sql.starts_with("WITH m AS MATERIALIZED (SELECT "));
        assert_eq!(sql.matches("tags @> ANY(").count(), 1);
        assert!(!sql.contains("tags @> $"));
        let fence_end = sql.find(") SELECT * FROM m ").expect("fence closes");
        assert!(!sql[..fence_end].contains("ORDER BY"));
        assert!(!sql[..fence_end].contains("LIMIT"));
        assert!(sql[..fence_end].contains("payload_bytes"));
        assert!(sql[fence_end..].contains("ORDER BY created_at DESC, id ASC LIMIT "));
    }
}
