//! Protected append-only SQLite spool for signed NIP-CB lifecycle events.

use std::path::Path;
use std::time::{Duration, Instant};

use nostr::{Event, JsonUtil};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};

use buzz_core_pkg::command_brief::MAX_EVENT_CONTENT_BYTES;
use buzz_core_pkg::kind::KIND_COMMAND_BRIEF;

mod schema;

pub use schema::validate_command_brief_store_schema;
use schema::{create_current_schema, CLAIMS_DEFERRED_INDEX_SQL, CLAIMS_TABLE_SQL, SCHEMA_VERSION};
const MAX_RETRIES: i64 = 8;
const MAX_RETRY_DELAY_SECONDS: i64 = 3_600;
const MAX_DUE_ROWS: usize = 64;

const MIGRATE_V1_TO_V2: &str = "
ALTER TABLE command_brief_spool ADD COLUMN append_sequence INTEGER;
CREATE UNIQUE INDEX command_brief_spool_sequence
ON command_brief_spool(owner_pubkey, run_id, append_sequence);
CREATE TABLE command_brief_heads (
    owner_pubkey       TEXT NOT NULL,
    run_id             TEXT NOT NULL,
    head_event_id      TEXT NOT NULL,
    head_sequence      INTEGER NOT NULL,
    PRIMARY KEY (owner_pubkey, run_id)
);
";

const MIGRATE_V2_TO_V3: &str = "
CREATE TABLE command_brief_schedule (
    schedule_id       TEXT PRIMARY KEY,
    classification    TEXT NOT NULL CHECK (classification = 'OFFICIAL'),
    enabled           INTEGER NOT NULL CHECK (enabled IN (0,1)),
    local_time        TEXT NOT NULL,
    timezone          TEXT NOT NULL,
    catch_up_same_day INTEGER NOT NULL CHECK (catch_up_same_day IN (0,1)),
    concurrency       INTEGER NOT NULL CHECK (concurrency IN (1,2)),
    updated_at        INTEGER NOT NULL
);
CREATE TABLE command_brief_schedule_claims (
    idempotency_key   TEXT PRIMARY KEY,
    schedule_id       TEXT NOT NULL,
    local_date        TEXT NOT NULL,
    timezone          TEXT NOT NULL,
    state             TEXT NOT NULL CHECK (state IN ('claimed','deferred','started','completed')),
    deferred_reason   TEXT,
    retry_count       INTEGER NOT NULL DEFAULT 0 CHECK (retry_count BETWEEN 0 AND 8),
    transition_token  TEXT,
    claimed_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    run_id            TEXT,
    UNIQUE (schedule_id, local_date)
);
CREATE INDEX command_brief_schedule_deferred
ON command_brief_schedule_claims(state, retry_count, updated_at);
";

/// Closed local relay-publication state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublishState {
    /// Durable locally and awaiting relay acceptance.
    Queued,
    /// Relay accepted this exact signed event.
    Published,
}

/// Borrowed fields for one already-signed spool insert.
pub struct SpoolInsert {
    /// Current unlocked owner identity.
    pub owner_pubkey: String,
    /// Lifecycle run identity.
    pub run_id: String,
    /// Signed event identity.
    pub event_id: String,
    /// Public lifecycle status tag.
    pub status: String,
    /// Exact predecessor event ID.
    pub previous_event_id: Option<String>,
    /// NIP-44 v2 event content.
    pub encrypted_payload: String,
    /// Exact signed event JSON used for idempotent republish.
    pub raw_event: String,
    /// Unix timestamp used for stable local ordering.
    pub created_at: i64,
}

/// One bounded due publication row.
pub struct DueSpoolEvent {
    /// Current unlocked owner.
    pub owner_pubkey: String,
    /// Run identity.
    pub run_id: String,
    /// Signed event ID.
    pub event_id: String,
    /// Exact signed event JSON.
    pub raw_event: String,
    /// Public lifecycle status.
    pub status: String,
    /// Exact predecessor event ID.
    pub previous_event_id: Option<String>,
    /// Exact encrypted event content.
    pub encrypted_payload: String,
    /// Signed event timestamp.
    pub created_at: i64,
    /// Current publication state.
    pub publish_state: PublishState,
    /// Bounded number of failed relay attempts.
    pub retry_count: i64,
    /// Earliest next attempt.
    pub next_retry_at: i64,
}

/// Open the protected spool with WAL and apply any required atomic migration.
///
/// Full integrity and exact-schema validation belongs to startup, migration,
/// backup, and restore boundaries rather than each operational connection.
pub fn open_command_brief_store(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| "command brief store unavailable")?;
        protect_path(parent, true)?;
    }
    let conn = Connection::open(path).map_err(|_| "command brief store unavailable")?;
    conn.pragma_update(None, "busy_timeout", 5_000)
        .map_err(|_| "command brief store unavailable")?;
    set_wal(&conn)?;
    migrate_command_brief_store(&conn)?;
    protect_path(path, false)?;
    Ok(conn)
}

/// Return whether the owner-bound audit spool has a durable terminal for a run.
pub fn has_spooled_terminal(
    conn: &Connection,
    owner_pubkey: &str,
    run_id: &str,
) -> Result<bool, String> {
    conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM command_brief_spool
             WHERE owner_pubkey=?1 AND run_id=?2
         )",
        params![owner_pubkey, run_id],
        |row| row.get(0),
    )
    .map_err(|_| "command brief spool unavailable".to_string())
}

/// Apply the schema as one exclusive transaction.
pub fn migrate_command_brief_store(conn: &Connection) -> Result<(), String> {
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|_| "command brief store migration failed")?;
    if version > SCHEMA_VERSION {
        return Err("command brief store migration failed".into());
    }
    if version == 0 {
        let tx = Transaction::new_unchecked(conn, TransactionBehavior::Exclusive)
            .map_err(|_| "command brief store migration failed")?;
        create_current_schema(&tx).map_err(|_| "command brief store migration failed")?;
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(|_| "command brief store migration failed")?;
        tx.commit()
            .map_err(|_| "command brief store migration failed")?;
    } else if version == 1 {
        migrate_v1_to_v2(conn)?;
        migrate_v2_to_v3(conn)?;
        migrate_v3_to_v4(conn)?;
    } else if version == 2 {
        migrate_v2_to_v3(conn)?;
        migrate_v3_to_v4(conn)?;
    } else if version == 3 {
        migrate_v3_to_v4(conn)?;
    }
    if version != SCHEMA_VERSION {
        validate_command_brief_store_schema(conn)
            .map_err(|_| "command brief store migration failed")?;
    }
    Ok(())
}

fn migrate_v1_to_v2(conn: &Connection) -> Result<(), String> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Exclusive)
        .map_err(|_| "command brief store migration failed")?;
    tx.execute_batch(MIGRATE_V1_TO_V2)
        .map_err(|_| "command brief store migration failed")?;
    let rows = {
        let mut statement = tx
            .prepare(
                "SELECT rowid,owner_pubkey,run_id FROM command_brief_spool
                 ORDER BY owner_pubkey,run_id,created_at,rowid",
            )
            .map_err(|_| "command brief store migration failed")?;
        let mapped = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|_| "command brief store migration failed")?;
        mapped
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "command brief store migration failed")?
    };
    let mut previous_key: Option<(String, String)> = None;
    let mut sequence = 0_i64;
    for (rowid, owner, run) in rows {
        let key = (owner, run);
        if previous_key.as_ref() != Some(&key) {
            sequence = 1;
            previous_key = Some(key);
        } else {
            sequence += 1;
        }
        tx.execute(
            "UPDATE command_brief_spool SET append_sequence=?2 WHERE rowid=?1",
            params![rowid, sequence],
        )
        .map_err(|_| "command brief store migration failed")?;
    }
    tx.execute_batch(
        "INSERT INTO command_brief_heads(owner_pubkey,run_id,head_event_id,head_sequence)
         SELECT spool.owner_pubkey,spool.run_id,spool.event_id,spool.append_sequence
         FROM command_brief_spool spool
         WHERE spool.append_sequence = (
             SELECT MAX(candidate.append_sequence)
             FROM command_brief_spool candidate
             WHERE candidate.owner_pubkey=spool.owner_pubkey
               AND candidate.run_id=spool.run_id
         );",
    )
    .map_err(|_| "command brief store migration failed")?;
    tx.pragma_update(None, "user_version", 2)
        .map_err(|_| "command brief store migration failed")?;
    tx.commit()
        .map_err(|_| "command brief store migration failed".to_string())
}

fn migrate_v2_to_v3(conn: &Connection) -> Result<(), String> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Exclusive)
        .map_err(|_| "command brief store migration failed")?;
    tx.execute_batch(MIGRATE_V2_TO_V3)
        .map_err(|_| "command brief store migration failed")?;
    tx.pragma_update(None, "user_version", 3)
        .map_err(|_| "command brief store migration failed")?;
    tx.commit()
        .map_err(|_| "command brief store migration failed".to_string())
}

fn migrate_v3_to_v4(conn: &Connection) -> Result<(), String> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Exclusive)
        .map_err(|_| "command brief store migration failed")?;
    tx.execute_batch(&CLAIMS_TABLE_SQL.replace(
        "command_brief_schedule_claims",
        "command_brief_schedule_claims_v4",
    ))
    .map_err(|_| "command brief store migration failed")?;
    let rows = {
        let mut statement = tx
            .prepare(
                "SELECT idempotency_key,schedule_id,local_date,timezone,state,
                        deferred_reason,retry_count,transition_token,claimed_at,updated_at,run_id
                 FROM command_brief_schedule_claims",
            )
            .map_err(|_| "command brief store migration failed")?;
        let mapped = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, Option<String>>(10)?,
                ))
            })
            .map_err(|_| "command brief store migration failed")?;
        mapped
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "command brief store migration failed")?
    };
    for (key, schedule, date, timezone, state, reason, retry, token, claimed, updated, run) in rows
    {
        let _previous_run_id = run;
        let run_id = format!("scheduled-{}", hex::encode(Sha256::digest(key.as_bytes())));
        tx.execute(
            "INSERT INTO command_brief_schedule_claims_v4(
                idempotency_key,schedule_id,local_date,timezone,state,deferred_reason,
                retry_count,transition_token,claimed_at,updated_at,run_id
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                key, schedule, date, timezone, state, reason, retry, token, claimed, updated,
                run_id
            ],
        )
        .map_err(|_| "command brief store migration failed")?;
    }
    tx.execute_batch(
        "DROP INDEX command_brief_schedule_deferred;
         DROP TABLE command_brief_schedule_claims;
         ALTER TABLE command_brief_schedule_claims_v4
             RENAME TO command_brief_schedule_claims;
         ",
    )
    .map_err(|_| "command brief store migration failed")?;
    tx.execute_batch(CLAIMS_DEFERRED_INDEX_SQL)
        .map_err(|_| "command brief store migration failed")?;
    tx.pragma_update(None, "user_version", SCHEMA_VERSION)
        .map_err(|_| "command brief store migration failed")?;
    tx.commit()
        .map_err(|_| "command brief store migration failed".to_string())
}

/// Append one exact signed event. Exact re-insertion is idempotent.
pub fn insert_spool_event(conn: &Connection, insert: SpoolInsert) -> Result<bool, String> {
    validate_insert(&insert)?;
    let event = Event::from_json(&insert.raw_event).map_err(|_| "command brief spool rejected")?;
    if event.id.to_hex() != insert.event_id
        || event.pubkey.to_hex() != insert.owner_pubkey
        || event.kind.as_u16() as u32 != KIND_COMMAND_BRIEF
        || event.content != insert.encrypted_payload
        || event.content.len() > MAX_EVENT_CONTENT_BYTES
        || event.created_at.as_secs() as i64 != insert.created_at
        || !event.verify_id()
        || !event.verify_signature()
        || !event_envelope_matches_insert(&event, &insert)
    {
        return Err("command brief spool rejected".into());
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .map_err(|_| "command brief spool unavailable")?;
    if let Some(existing) = tx
        .query_row(
            "SELECT raw_event FROM command_brief_spool
             WHERE owner_pubkey = ?1 AND event_id = ?2",
            params![&insert.owner_pubkey, &insert.event_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| "command brief spool unavailable")?
    {
        if existing == insert.raw_event {
            tx.commit().map_err(|_| "command brief spool unavailable")?;
            return Ok(false);
        }
        return Err("command brief spool conflict".into());
    }
    let head: Option<(String, i64)> = tx
        .query_row(
            "SELECT head_event_id,head_sequence FROM command_brief_heads
             WHERE owner_pubkey = ?1 AND run_id = ?2",
            params![&insert.owner_pubkey, &insert.run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|_| "command brief spool unavailable")?;
    if head.as_ref().map(|(event_id, _)| event_id.as_str()) != insert.previous_event_id.as_deref() {
        return Err("command brief spool predecessor conflict".into());
    }
    let append_sequence = head.map_or(1, |(_, sequence)| sequence.saturating_add(1));
    tx.execute(
        "INSERT INTO command_brief_spool
         (owner_pubkey,run_id,event_id,status,previous_event_id,encrypted_payload,
          raw_event,publish_state,retry_count,next_retry_at,created_at,append_sequence)
         VALUES (?1,?2,?3,?4,?5,?6,?7,'queued',0,0,?8,?9)",
        params![
            &insert.owner_pubkey,
            &insert.run_id,
            &insert.event_id,
            &insert.status,
            &insert.previous_event_id,
            &insert.encrypted_payload,
            &insert.raw_event,
            insert.created_at,
            append_sequence
        ],
    )
    .map_err(|_| "command brief spool conflict")?;
    tx.execute(
        "INSERT INTO command_brief_heads(owner_pubkey,run_id,head_event_id,head_sequence)
         VALUES (?1,?2,?3,?4)
         ON CONFLICT(owner_pubkey,run_id) DO UPDATE SET
           head_event_id=excluded.head_event_id,
           head_sequence=excluded.head_sequence",
        params![
            &insert.owner_pubkey,
            &insert.run_id,
            &insert.event_id,
            append_sequence
        ],
    )
    .map_err(|_| "command brief spool conflict")?;
    tx.commit().map_err(|_| "command brief spool unavailable")?;
    Ok(true)
}

fn event_envelope_matches_insert(event: &Event, insert: &SpoolInsert) -> bool {
    let mut owner = None;
    let mut run = None;
    let mut status = None;
    let mut previous = None;
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.len() != 2 {
            return false;
        }
        let target = match parts[0].as_str() {
            "p" => &mut owner,
            "d" => &mut run,
            "status" => &mut status,
            "previous" => &mut previous,
            _ => return false,
        };
        if target.replace(parts[1].as_str()).is_some() {
            return false;
        }
    }
    owner == Some(insert.owner_pubkey.as_str())
        && run == Some(insert.run_id.as_str())
        && status == Some(insert.status.as_str())
        && previous == insert.previous_event_id.as_deref()
}

/// Return a bounded deterministic batch due for idempotent republish.
pub fn list_due_spool_events(
    conn: &Connection,
    owner_pubkey: &str,
    now: i64,
    limit: usize,
) -> Result<Vec<DueSpoolEvent>, String> {
    let limit = limit.min(MAX_DUE_ROWS) as i64;
    let mut statement = conn
        .prepare(
            "SELECT owner_pubkey,run_id,event_id,raw_event,status,previous_event_id,
                    encrypted_payload,created_at,publish_state,retry_count,next_retry_at
             FROM command_brief_spool
             WHERE owner_pubkey = ?1 AND publish_state = 'queued'
               AND retry_count < 8 AND next_retry_at <= ?2
             ORDER BY append_sequence,event_id LIMIT ?3",
        )
        .map_err(|_| "command brief spool unavailable")?;
    let rows = statement
        .query_map(params![owner_pubkey, now, limit], |row| {
            Ok(DueSpoolEvent {
                owner_pubkey: row.get(0)?,
                run_id: row.get(1)?,
                event_id: row.get(2)?,
                raw_event: row.get(3)?,
                status: row.get(4)?,
                previous_event_id: row.get(5)?,
                encrypted_payload: row.get(6)?,
                created_at: row.get(7)?,
                publish_state: parse_state(&row.get::<_, String>(8)?)?,
                retry_count: row.get(9)?,
                next_retry_at: row.get(10)?,
            })
        })
        .map_err(|_| "command brief spool unavailable")?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|_| "command brief spool unavailable".into())
}

/// Rearm a bounded queued batch on one explicit relay-readiness transition.
///
/// Rows permanently rejected by local validation are never rearmed.
pub fn rearm_queued_spool_events(
    conn: &Connection,
    owner_pubkey: &str,
    now: i64,
    limit: usize,
) -> Result<usize, String> {
    let limit = limit.min(MAX_DUE_ROWS) as i64;
    let event_ids = {
        let mut statement = conn
            .prepare(
                "SELECT event_id FROM command_brief_spool
                 WHERE owner_pubkey=?1 AND publish_state='queued'
                   AND (last_error_code IS NULL OR last_error_code='relay_unavailable')
                 ORDER BY append_sequence,event_id LIMIT ?2",
            )
            .map_err(|_| "command brief spool unavailable")?;
        let rows = statement
            .query_map(params![owner_pubkey, limit], |row| row.get::<_, String>(0))
            .map_err(|_| "command brief spool unavailable")?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|_| "command brief spool unavailable")?
    };
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .map_err(|_| "command brief spool unavailable")?;
    for event_id in &event_ids {
        tx.execute(
            "UPDATE command_brief_spool
             SET retry_count=0,next_retry_at=?3,last_error_code=NULL
             WHERE owner_pubkey=?1 AND event_id=?2 AND publish_state='queued'",
            params![owner_pubkey, event_id, now],
        )
        .map_err(|_| "command brief spool unavailable")?;
    }
    tx.commit().map_err(|_| "command brief spool unavailable")?;
    Ok(event_ids.len())
}

/// Parse and revalidate every signed field before exact-ID republish.
pub fn validate_due_spool_event(row: &DueSpoolEvent) -> Result<Event, String> {
    let insert = SpoolInsert {
        owner_pubkey: row.owner_pubkey.clone(),
        run_id: row.run_id.clone(),
        event_id: row.event_id.clone(),
        status: row.status.clone(),
        previous_event_id: row.previous_event_id.clone(),
        encrypted_payload: row.encrypted_payload.clone(),
        raw_event: row.raw_event.clone(),
        created_at: row.created_at,
    };
    validate_insert(&insert)?;
    let event = Event::from_json(&row.raw_event).map_err(|_| "command brief spool rejected")?;
    if event.id.to_hex() != row.event_id
        || event.pubkey.to_hex() != row.owner_pubkey
        || event.kind.as_u16() as u32 != KIND_COMMAND_BRIEF
        || event.content != row.encrypted_payload
        || event.content.len() > MAX_EVENT_CONTENT_BYTES
        || event.created_at.as_secs() as i64 != row.created_at
        || !event.verify_id()
        || !event.verify_signature()
        || !event_envelope_matches_insert(&event, &insert)
    {
        return Err("command brief spool rejected".into());
    }
    Ok(event)
}

/// Return the latest append-only lifecycle head for one owner/run.
pub fn latest_event_id(
    conn: &Connection,
    owner_pubkey: &str,
    run_id: &str,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT head_event_id FROM command_brief_heads
         WHERE owner_pubkey=?1 AND run_id=?2",
        params![owner_pubkey, run_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|_| "command brief spool unavailable".into())
}

/// Mark exact relay acceptance without changing event bytes.
pub fn mark_published(
    conn: &Connection,
    owner_pubkey: &str,
    event_id: &str,
    published_at: i64,
) -> Result<(), String> {
    let changed = conn
        .execute(
            "UPDATE command_brief_spool
             SET publish_state='published',published_at=?3,last_error_code=NULL
             WHERE owner_pubkey=?1 AND event_id=?2",
            params![owner_pubkey, event_id, published_at],
        )
        .map_err(|_| "command brief spool unavailable")?;
    if changed == 1 {
        Ok(())
    } else {
        Err("command brief spool row missing".into())
    }
}

/// Record one bounded failed attempt and exponential retry time.
pub fn mark_publish_failed(
    conn: &Connection,
    owner_pubkey: &str,
    event_id: &str,
    now: i64,
) -> Result<(), String> {
    let retry: Option<i64> = conn
        .query_row(
            "SELECT retry_count FROM command_brief_spool
             WHERE owner_pubkey=?1 AND event_id=?2 AND publish_state='queued'",
            params![owner_pubkey, event_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| "command brief spool unavailable")?;
    let retry = retry.ok_or_else(|| "command brief spool row missing".to_string())?;
    let next_retry = (retry + 1).min(MAX_RETRIES);
    let exponent = u32::try_from(next_retry.min(12)).unwrap_or(12);
    let delay = (1_i64 << exponent).min(MAX_RETRY_DELAY_SECONDS);
    conn.execute(
        "UPDATE command_brief_spool
         SET retry_count=?3,next_retry_at=?4,last_error_code='relay_unavailable'
         WHERE owner_pubkey=?1 AND event_id=?2 AND publish_state='queued'",
        params![
            owner_pubkey,
            event_id,
            next_retry,
            now.saturating_add(delay)
        ],
    )
    .map_err(|_| "command brief spool unavailable")?;
    Ok(())
}

/// Permanently quarantine a locally invalid row so readiness cannot hot-loop it.
pub fn mark_publish_permanent(
    conn: &Connection,
    owner_pubkey: &str,
    event_id: &str,
) -> Result<(), String> {
    let changed = conn
        .execute(
            "UPDATE command_brief_spool
             SET retry_count=?3,next_retry_at=?4,last_error_code='invalid_event'
             WHERE owner_pubkey=?1 AND event_id=?2 AND publish_state='queued'",
            params![owner_pubkey, event_id, MAX_RETRIES, i64::MAX],
        )
        .map_err(|_| "command brief spool unavailable")?;
    if changed == 1 {
        Ok(())
    } else {
        Err("command brief spool row missing".into())
    }
}

fn parse_state(value: &str) -> rusqlite::Result<PublishState> {
    match value {
        "queued" => Ok(PublishState::Queued),
        "published" => Ok(PublishState::Published),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn validate_insert(insert: &SpoolInsert) -> Result<(), String> {
    let valid_hex = |value: &str| {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    };
    if !valid_hex(&insert.owner_pubkey)
        || !valid_hex(&insert.event_id)
        || insert.run_id.is_empty()
        || insert.run_id.len() > 256
        || !matches!(
            insert.status.as_str(),
            "completed" | "degraded" | "cancelled" | "failed"
        )
        || insert
            .previous_event_id
            .as_deref()
            .is_some_and(|value| !valid_hex(value))
        || insert.encrypted_payload.is_empty()
        || insert.encrypted_payload.len() > MAX_EVENT_CONTENT_BYTES
        || insert.raw_event.len() > MAX_EVENT_CONTENT_BYTES.saturating_mul(2)
    {
        return Err("command brief spool rejected".into());
    }
    Ok(())
}

fn set_wal(conn: &Connection) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match conn.pragma_update(None, "journal_mode", "WAL") {
            Ok(()) => return Ok(()),
            Err(error) if is_busy(&error) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => return Err("command brief store unavailable".into()),
        }
    }
}

fn is_busy(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(inner, _)
            if matches!(
                inner.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            )
    )
}

#[cfg(unix)]
fn protect_path(path: &Path, directory: bool) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if directory { 0o700 } else { 0o600 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|_| "command brief store protection failed".to_string())
}

#[cfg(not(unix))]
fn protect_path(_path: &Path, _directory: bool) -> Result<(), String> {
    Ok(())
}
