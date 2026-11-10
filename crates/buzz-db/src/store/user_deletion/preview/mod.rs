//! Observational batch planning. No mutation executor or durable progress exists here.
//!
//! A repeatable-read, read-only snapshot supplies rows. An in-memory set models
//! rows already emitted, so parents can follow children without deleting either.

use super::{normalize_user_deletion_pubkey, UserDeletionStore};
use crate::{DbError, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::PgConnection;
use std::collections::HashSet;
use std::io::Write;
use uuid::Uuid;

#[cfg(test)]
mod postgres_tests;
mod rows;

const KINDS: &[i32] = &[
    1, 7, 9, 30023, 40001, 40002, 40003, 40004, 40005, 40006, 40007, 40008, 45001, 45002, 45003,
];
const MAX_DEPTH: usize = 64;
const PAGE_SIZE: i64 = 100;
const EVENT_COLUMNS: &str = "e.id,e.created_at,e.channel_id,e.kind,
    (e.content ~* 'https?://|/media/' OR EXISTS (
      SELECT 1 FROM jsonb_array_elements(e.tags) t
      WHERE t->>0 IN ('imeta','link-preview','url','thumb','image','x','emoji'))) AS has_media";

/// Input to the standalone preview; never authorizes a deletion.
pub struct PreviewOptions {
    /// Explicit tenant scope, including content from former memberships.
    pub community: Uuid,
    /// Human public key, as npub or hex.
    pub pubkey: String,
    /// Optional trusted relay public key for historical actor/p attribution.
    pub relay_pubkey: Option<String>,
    /// Maximum explicit row operations in each printed batch (1–100).
    pub batch_size: u16,
    /// Maximum unique operations retained in the simulation (1–1,000,000).
    /// Exceeding this fails visibly; it never returns a complete partial plan.
    pub max_items: u32,
}

/// Final marker proving that all batches in the declared scope were printed.
#[derive(Debug, Serialize)]
pub struct PreviewSummary {
    /// Always `preview_complete`; absent if the preview fails or is interrupted.
    pub record: &'static str,
    /// Number of batches printed.
    pub batches: u64,
    /// Number of unique proposed row operations.
    pub items: usize,
    /// Operations carrying a media, active-work or delivery warning.
    pub warnings: usize,
    /// Authored event kinds outside this handler, with counts.
    pub excluded_event_kinds: Vec<Value>,
}

#[derive(Clone, sqlx::FromRow)]
struct Event {
    id: Vec<u8>,
    created_at: DateTime<Utc>,
    channel_id: Option<Uuid>,
    kind: i32,
    has_media: bool,
}

#[derive(Default, Clone)]
struct Cursor(Option<DateTime<Utc>>, Option<Vec<u8>>);

impl Cursor {
    fn advance(&mut self, event: &Event) {
        self.0 = Some(event.created_at);
        self.1 = Some(event.id.clone());
    }
}

struct Frame {
    event: Event,
    children: Cursor,
    reason: Value,
}

struct Output<'a, W> {
    writer: &'a mut W,
    community: Uuid,
    limit: usize,
    max_items: usize,
    seen: HashSet<String>,
    events: HashSet<(Vec<u8>, DateTime<Utc>)>,
    runs: HashSet<Uuid>,
    batch: Vec<Value>,
    repairs: Vec<Value>,
    batches: u64,
    warnings: usize,
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
        line(
            self.writer,
            &json!({
                "record": "batch", "number": self.batches, "community_id": self.community,
                "items": self.batch, "thread_repairs": self.repairs,
            }),
        )?;
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
    pub async fn preview_message_batches(
        &self,
        options: &PreviewOptions,
        writer: &mut impl Write,
    ) -> Result<PreviewSummary> {
        if !(1..=100).contains(&options.batch_size) || !(1..=1_000_000).contains(&options.max_items)
        {
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
        let mut tx = snapshot(&self.pool).await?;
        let host: String = sqlx::query_scalar("SELECT host FROM communities WHERE id=$1")
            .bind(options.community)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| invalid("unknown community"))?;
        line(
            writer,
            &json!({
                "record": "preview_start", "version": 1, "read_only": true,
                "community_id": options.community, "host": host, "pubkey": subject,
                "trusted_relay_pubkey": relay, "included_event_kinds": KINDS,
                "batch_size": options.batch_size, "max_items": options.max_items,
                "scope": "human content events, permitted dependents, reaction rows and workflow copies",
                "relay_attribution": if relay.is_some() { "enabled" } else { "not included; supply --relay-pubkey for relay-authored history" },
                "execution": "unavailable; media, worker drain, write fences and thread repair require separate validation",
                "thread_repairs": "reported separately from row items; metadata mutations are not expanded",
            }),
        )?;
        let mut output = Output {
            writer,
            community: options.community,
            limit: usize::from(options.batch_size),
            max_items: options.max_items as usize,
            seen: HashSet::new(),
            events: HashSet::new(),
            runs: HashSet::new(),
            batch: Vec::new(),
            repairs: Vec::new(),
            batches: 0,
            warnings: 0,
        };
        let mut cursor = Cursor::default();
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
                .bind(&subject)
                .bind(&relay)
                .bind(KINDS)
                .bind(cursor.0)
                .bind(&cursor.1)
                .fetch_all(&mut *tx)
                .await?;
            if events.is_empty() {
                break;
            }
            for event in events {
                cursor.advance(&event);
                walk(&mut tx, event, &mut output).await?;
            }
        }
        rows::orphan_sweeps(&mut tx, &subject, &mut output).await?;
        let query = format!(
            "WITH authored AS ({}) SELECT jsonb_build_object('kind',kind,'count',count(*))
            FROM authored WHERE NOT(kind=ANY($4)) GROUP BY kind ORDER BY kind",
            include_str!("authored.sql")
        );
        let excluded: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(query))
            .bind(options.community)
            .bind(&subject)
            .bind(&relay)
            .bind(KINDS)
            .fetch_all(&mut *tx)
            .await?;
        output.flush()?;
        let summary = PreviewSummary {
            record: "preview_complete",
            batches: output.batches,
            items: output.seen.len(),
            warnings: output.warnings,
            excluded_event_kinds: excluded,
        };
        // End the observational snapshot before claiming complete output.
        tx.rollback().await?;
        line(output.writer, &summary)?;
        Ok(summary)
    }
}

async fn walk(
    conn: &mut PgConnection,
    seed: Event,
    output: &mut Output<'_, impl Write>,
) -> Result<()> {
    if output.events.contains(&(seed.id.clone(), seed.created_at)) {
        return Ok(());
    }
    let mut stack = vec![Frame {
        event: seed,
        children: Cursor::default(),
        reason: json!({"type": "authored_event"}),
    }];
    while let Some(frame) = stack.last_mut() {
        let query = format!("SELECT {EVENT_COLUMNS} FROM events e CROSS JOIN LATERAL (
            SELECT decode(t->>1,'hex') AS id FROM jsonb_array_elements(e.tags) WITH ORDINALITY a(t,n)
            WHERE t->>0='e' AND t->>1 ~ '^[0-9a-fA-F]{{64}}$'
            ORDER BY CASE WHEN e.kind IN (40003,45002) THEN n ELSE -n END LIMIT 1) target
            WHERE e.community_id=$1 AND e.kind IN (7,40003,45002) AND target.id=$2
              AND e.channel_id IS NOT DISTINCT FROM $3::uuid
              AND ($4::timestamptz IS NULL OR (e.created_at,e.id)>($4,$5))
            ORDER BY e.created_at,e.id LIMIT 1");
        let child: Option<Event> = sqlx::query_as(sqlx::AssertSqlSafe(query))
            .bind(output.community)
            .bind(&frame.event.id)
            .bind(frame.event.channel_id)
            .bind(frame.children.0)
            .bind(&frame.children.1)
            .fetch_optional(&mut *conn)
            .await?;
        if let Some(child) = child {
            frame.children.advance(&child);
            let reason =
                json!({"type": "dependent_event", "target_event_id": hex::encode(&frame.event.id)});
            if stack.iter().any(|f| f.event.id == child.id) {
                return Err(invalid("cyclic event dependencies; preview is incomplete"));
            }
            if output
                .events
                .contains(&(child.id.clone(), child.created_at))
            {
                continue;
            }
            if stack.len() >= MAX_DEPTH {
                return Err(invalid(
                    "event dependency depth exceeds 64; preview is incomplete",
                ));
            }
            stack.push(Frame {
                event: child,
                children: Cursor::default(),
                reason,
            });
        } else {
            let frame = stack
                .pop()
                .ok_or_else(|| invalid("missing preview frame"))?;
            rows::event_rows(conn, &frame.event, output).await?;
            let roots: Vec<Value> = sqlx::query_scalar("SELECT jsonb_build_object(
                'channel_id',channel_id,'root_event_id',encode(COALESCE(root_event_id,event_id),'hex'))
                FROM thread_metadata WHERE community_id=$1 AND event_id=$2 AND event_created_at=$3
                ORDER BY channel_id,root_event_id")
                .bind(output.community).bind(&frame.event.id).bind(frame.event.created_at)
                .fetch_all(&mut *conn).await?;
            output.repairs.push(json!({"event_id": hex::encode(&frame.event.id),
                "channel_id": frame.event.channel_id, "stored_threads": roots,
                "action": "recompute affected thread metadata; preserve surviving reply ancestry"}));
            output.add(
                "events",
                json!({"id": hex::encode(&frame.event.id), "created_at": frame.event.created_at}),
                "would_delete",
                &json!({"selection": frame.reason, "kind": frame.event.kind}),
                frame
                    .event
                    .has_media
                    .then(|| "media references require prior cleanup".to_owned()),
            )?;
            output
                .events
                .insert((frame.event.id, frame.event.created_at));
        }
    }
    Ok(())
}
