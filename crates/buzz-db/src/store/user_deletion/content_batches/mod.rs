//! Shared user and owned-agent content batch planning with explicit preview and execution APIs.
//!
//! A repeatable-read, read-only snapshot supplies rows. An in-memory set models
//! rows already emitted without changing the source data during planning.

use super::{normalize_user_deletion_pubkey, UserDeletionStore};
use crate::{DbError, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::PgConnection;
use std::collections::HashSet;
use std::io::Write;
use uuid::Uuid;

mod execute;
mod grouped;
mod owned;
mod reference_index;
mod retention;
mod rows;

const KINDS: &[i32] = &[
    5, 41010, 41011, 41012, 46020, 46030, 46031, 3, 10000, 10001, 10002, 10003, 30000, 30003,
    30315, 30078, 30300, 30174, 44200, 43001, 43002, 43003, 43004, 43005, 43006, 30620, 30350, 1,
    7, 9, 30023, 40001, 40002, 40003, 40004, 40005, 40006, 40007, 40008, 45001, 45002, 45003,
];
const PAGE_SIZE: i64 = 100;
const MAX_TARGETS: usize = 10_000;
const EVENT_COLUMNS: &str = "e.id,e.created_at,e.channel_id,e.kind,e.tags";

/// Scope and bounds for standalone preview or explicitly requested deletion.
pub struct ContentBatchOptions {
    /// Explicit tenant scope, including content from former memberships.
    pub community: Uuid,
    /// Human public key, as npub or hex; includes directly owned agents.
    pub pubkey: String,
    /// Optional trusted relay public key for historical actor/p attribution.
    pub relay_pubkey: Option<String>,
    /// Maximum explicit row operations in each printed batch (1–100).
    pub batch_size: u16,
    /// Maximum unique operations retained in the simulation (1–1,000,000).
    /// Exceeding this fails visibly; it never returns a complete partial plan.
    pub max_items: u32,
}

/// Final marker for the declared scope; never claims complete account erasure.
#[derive(Debug, Serialize)]
pub struct ContentBatchSummary {
    /// `preview_complete` or `delete_complete`; absent on failure/interruption.
    pub record: &'static str,
    /// Number of batches printed.
    pub batches: u64,
    /// Number of unique proposed row operations.
    pub items: usize,
    /// Operations carrying an active-work or delivery warning.
    pub warnings: usize,
    /// Events retained as erased structural tombstones.
    pub scrubbed_events: usize,
    /// Authored event kinds outside this handler, with counts.
    pub excluded_event_kinds: Vec<Value>,
}

#[derive(Clone, sqlx::FromRow, Serialize)]
struct Event {
    id: Vec<u8>,
    created_at: DateTime<Utc>,
    channel_id: Option<Uuid>,
    kind: i32,
    tags: Value,
}

#[derive(Default, Clone)]
struct Cursor(Option<DateTime<Utc>>, Option<Vec<u8>>);

impl Cursor {
    fn advance(&mut self, event: &Event) {
        self.0 = Some(event.created_at);
        self.1 = Some(event.id.clone());
    }
}

struct Output<'a, W> {
    writer: &'a mut W,
    community: Uuid,
    limit: usize,
    max_items: usize,
    seen: HashSet<String>,
    batch: Vec<Value>,
    repairs: Vec<Value>,
    batches: u64,
    warnings: usize,
    collected: Option<Vec<Value>>,
}

fn invalid(message: impl Into<String>) -> DbError {
    DbError::DeletionSafety(message.into())
}

fn line(writer: &mut impl Write, value: &impl Serialize) -> Result<()> {
    serde_json::to_writer(&mut *writer, value)?;
    writer
        .write_all(b"\n")
        .and_then(|_| writer.flush())
        .map_err(|error| invalid(format!("preview output failed: {error}")))
}

async fn snapshot(pool: &sqlx::PgPool) -> Result<sqlx::Transaction<'_, sqlx::Postgres>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL statement_timeout = '30s'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL lock_timeout = '3s'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL TIME ZONE 'UTC'")
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

impl<W: Write> Output<'_, W> {
    fn add(
        &mut self,
        table: &str,
        key: Value,
        action: &str,
        reason: &Value,
        warning: Option<String>,
    ) -> Result<()> {
        let identity = format!("{table}:{key}");
        if self.seen.contains(&identity) {
            return Ok(());
        }
        if self.seen.len() >= self.max_items {
            return Err(invalid(
                "preview item limit exceeded; output is incomplete; increase --max-items",
            ));
        }
        self.seen.insert(identity);
        self.warnings += usize::from(warning.is_some());
        self.batch.push(json!({
            "table": table, "key": key, "action": action,
            "reason": reason, "warning": warning,
        }));
        if self.batch.len() == self.limit {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        if self.batch.is_empty() {
            return Ok(());
        }
        self.batches += 1;
        let batch = json!({
            "record": "batch", "number": self.batches, "community_id": self.community,
            "items": self.batch, "thread_repairs": self.repairs,
        });
        if let Some(collected) = &mut self.collected {
            collected.push(batch);
        } else {
            line(self.writer, &batch)?;
        }
        self.batch.clear();
        self.repairs.clear();
        Ok(())
    }
}

impl UserDeletionStore {
    /// Print JSON lines for every proposed batch in one read-only snapshot.
    ///
    /// This does not save a request, acquire a worker lease, update counters,
    /// migrate the schema, or delete rows. The completion marker describes only
    /// the stated handler scope; it is not evidence that deletion is safe to run.
    async fn preview_message_batches(
        &self,
        options: &ContentBatchOptions,
        writer: &mut impl Write,
    ) -> Result<ContentBatchSummary> {
        let mut tx = snapshot(&self.pool).await?;
        let plan = plan(&mut tx, options, writer, false).await?;
        tx.rollback().await?;
        line(writer, &plan.summary)?;
        Ok(plan.summary)
    }
}

struct Plan {
    start: Value,
    summary: ContentBatchSummary,
    batches: Vec<Value>,
}

async fn plan(
    conn: &mut PgConnection,
    options: &ContentBatchOptions,
    writer: &mut impl Write,
    collect: bool,
) -> Result<Plan> {
    reference_index::ensure(conn).await?;
    if !(1..=100).contains(&options.batch_size) || !(1..=1_000_000).contains(&options.max_items) {
        return Err(invalid(
            "batch size must be 1–100 and max items must be 1–1,000,000",
        ));
    }
    let subject = normalize_user_deletion_pubkey(&options.pubkey)?;
    let relay = options
        .relay_pubkey
        .as_deref()
        .map(normalize_user_deletion_pubkey)
        .transpose()?;
    if relay.as_deref() == Some(&subject) {
        return Err(invalid(
            "the human and trusted relay public keys must differ",
        ));
    }
    // Same ownership lookup as media cleanup: discover directly owned keys
    // across communities, but select content only in options.community.
    let targets: Vec<Vec<u8>> = sqlx::query_scalar(
        "SELECT key FROM (
            SELECT decode($1, 'hex') AS key
            UNION SELECT pubkey FROM users WHERE agent_owner_pubkey=decode($1, 'hex')
         ) targets ORDER BY key LIMIT $2",
    )
    .bind(&subject)
    .bind((MAX_TARGETS + 1) as i64)
    .fetch_all(&mut *conn)
    .await?;
    if targets.len() > MAX_TARGETS {
        return Err(invalid("content target key limit exceeded (10,000)"));
    }
    let target_pubkeys: Vec<String> = targets.iter().map(hex::encode).collect();
    if relay
        .as_ref()
        .is_some_and(|key| target_pubkeys.contains(key))
    {
        return Err(invalid("target and trusted relay public keys must differ"));
    }
    let host: String = sqlx::query_scalar("SELECT host FROM communities WHERE id=$1")
        .bind(options.community)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or_else(|| invalid("unknown community"))?;
    let start = json!({
        "record": "preview_start", "version": 1, "read_only": true,
        "community_id": options.community, "host": host, "pubkey": subject,
        "target_pubkeys": target_pubkeys, "trusted_relay_pubkey": relay, "included_event_kinds": KINDS,
        "batch_size": options.batch_size, "max_items": options.max_items,
        "scope": "user- and owned-agent-attributed events and reactions, owned subscriptions and notification delivery",
        "preserved": "other users' events and reactions; workflow database records and other owners' delivery logs",
        "relay_attribution": if relay.is_some() { "enabled" } else { "not included; supply --relay-pubkey for relay-authored history" },
        "execution": "preview only; --delete prints and commits batches after execution checks",
        "thread_repairs": "reported separately from row items; metadata mutations are not expanded",
    });
    if !collect {
        line(writer, &start)?;
    }
    let mut output = Output {
        writer,
        community: options.community,
        limit: usize::from(options.batch_size),
        max_items: options.max_items as usize,
        seen: HashSet::new(),
        batch: Vec::new(),
        repairs: Vec::new(),
        batches: 0,
        warnings: 0,
        collected: collect.then(Vec::new),
    };
    owned::plan(conn, &targets, &mut output).await?;
    rows::authored_reactions(conn, &targets, &mut output).await?;
    let mut cursor = Cursor::default();
    let mut selected = Vec::new();
    loop {
        let query = format!(
            "WITH authored AS ({}) SELECT {EVENT_COLUMNS} FROM authored e
                WHERE e.kind=ANY($4) AND ($5::timestamptz IS NULL OR (e.created_at,e.id)>($5,$6))
                ORDER BY e.created_at,e.id LIMIT 100",
            include_str!("authored.sql")
        );
        // Only static SQL fragments are interpolated; every value is bound.
        let events: Vec<Event> = sqlx::query_as(sqlx::AssertSqlSafe(query))
            .bind(options.community)
            .bind(&targets)
            .bind(&relay)
            .bind(KINDS)
            .bind(cursor.0)
            .bind(&cursor.1)
            .fetch_all(&mut *conn)
            .await?;
        if events.is_empty() {
            break;
        }
        for event in events {
            cursor.advance(&event);
            selected.push(event);
            if selected.len() > options.max_items as usize {
                return Err(invalid("content event limit exceeded"));
            }
        }
    }
    let needed = retention::needed(conn, options.community, &selected, &targets).await?;
    // Children before ancestors: a later batch must not expose a live child
    // whose selected ancestor was already physically removed.
    let metadata = retention::metadata(conn, options.community, &selected).await?;
    let selected = retention::children_first(selected, &metadata)?;
    for page in selected.chunks(PAGE_SIZE as usize) {
        rows::event_rows(conn, page, &mut output).await?;
        for event in page {
            let id = hex::encode(&event.id);
            let scrub = needed.contains(&id);
            output
                .repairs
                .push(json!({"event_id": id, "channel_id": event.channel_id,
                "metadata": metadata.get(&id), "action": "preserve ancestry and update counts"}));
            output.add(
                "events",
                json!({"id": id, "created_at": event.created_at}),
                if scrub { "would_scrub" } else { "would_delete" },
                &json!({"selection": {"type": "authored_event"}, "kind": event.kind}),
                None,
            )?;
        }
    }
    let query = format!(
        "WITH authored AS ({}) SELECT jsonb_build_object('kind',kind,'count',count(*))
            FROM authored WHERE NOT(kind=ANY($4)) GROUP BY kind ORDER BY kind",
        include_str!("authored.sql")
    );
    let excluded: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(query))
        .bind(options.community)
        .bind(&targets)
        .bind(&relay)
        .bind(KINDS)
        .fetch_all(&mut *conn)
        .await?;
    output.flush()?;
    let summary = ContentBatchSummary {
        record: "preview_complete",
        batches: output.batches,
        items: output.seen.len(),
        warnings: output.warnings,
        scrubbed_events: needed.len(),
        excluded_event_kinds: excluded,
    };
    Ok(Plan {
        start,
        summary,
        batches: output.collected.unwrap_or_default(),
    })
}

#[cfg(test)]
mod postgres_tests;

#[cfg(test)]
mod agents_postgres_tests;

#[cfg(test)]
mod scrub_postgres_tests;

#[cfg(test)]
mod reference_index_postgres_tests;
