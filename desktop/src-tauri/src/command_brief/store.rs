//! Protected append-only SQLite spool for signed NIP-CB lifecycle events.

use std::path::Path;
use std::time::{Duration, Instant};

use nostr::{Event, JsonUtil};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

const SCHEMA_VERSION: i64 = 1;
const MAX_RETRIES: i64 = 8;
const MAX_RETRY_DELAY_SECONDS: i64 = 3_600;
const MAX_DUE_ROWS: usize = 64;

const SCHEMA: &str = "
CREATE TABLE command_brief_spool (
    owner_pubkey       TEXT NOT NULL,
    run_id             TEXT NOT NULL,
    event_id           TEXT NOT NULL,
    status             TEXT NOT NULL,
    previous_event_id  TEXT,
    encrypted_payload  TEXT NOT NULL,
    raw_event           TEXT NOT NULL,
    publish_state       TEXT NOT NULL CHECK (publish_state IN ('queued','published')),
    retry_count         INTEGER NOT NULL DEFAULT 0 CHECK (retry_count BETWEEN 0 AND 8),
    next_retry_at       INTEGER NOT NULL DEFAULT 0,
    last_error_code     TEXT,
    created_at          INTEGER NOT NULL,
    published_at        INTEGER,
    PRIMARY KEY (owner_pubkey, run_id, event_id),
    UNIQUE (owner_pubkey, event_id)
);
CREATE INDEX command_brief_spool_due
ON command_brief_spool(owner_pubkey, publish_state, next_retry_at, created_at);
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
    /// Current publication state.
    pub publish_state: PublishState,
    /// Bounded number of failed relay attempts.
    pub retry_count: i64,
    /// Earliest next attempt.
    pub next_retry_at: i64,
}

/// Open the protected spool with WAL and apply its atomic schema migration.
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
        tx.execute_batch(SCHEMA)
            .map_err(|_| "command brief store migration failed")?;
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(|_| "command brief store migration failed")?;
        tx.commit()
            .map_err(|_| "command brief store migration failed")?;
    }
    Ok(())
}

/// Append one exact signed event. Exact re-insertion is idempotent.
pub fn insert_spool_event(conn: &Connection, insert: SpoolInsert) -> Result<bool, String> {
    validate_insert(&insert)?;
    let event = Event::from_json(&insert.raw_event).map_err(|_| "command brief spool rejected")?;
    if event.id.to_hex() != insert.event_id
        || event.pubkey.to_hex() != insert.owner_pubkey
        || event.content != insert.encrypted_payload
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
    let latest: Option<String> = tx
        .query_row(
            "SELECT event_id FROM command_brief_spool
             WHERE owner_pubkey = ?1 AND run_id = ?2
             ORDER BY created_at DESC, rowid DESC LIMIT 1",
            params![&insert.owner_pubkey, &insert.run_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| "command brief spool unavailable")?;
    if latest.as_deref() != insert.previous_event_id.as_deref() {
        return Err("command brief spool predecessor conflict".into());
    }
    tx.execute(
        "INSERT INTO command_brief_spool
         (owner_pubkey,run_id,event_id,status,previous_event_id,encrypted_payload,
          raw_event,publish_state,retry_count,next_retry_at,created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,'queued',0,0,?8)",
        params![
            &insert.owner_pubkey,
            &insert.run_id,
            &insert.event_id,
            &insert.status,
            &insert.previous_event_id,
            &insert.encrypted_payload,
            &insert.raw_event,
            insert.created_at
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
            "SELECT owner_pubkey,run_id,event_id,raw_event,publish_state,retry_count,next_retry_at
             FROM command_brief_spool
             WHERE owner_pubkey = ?1 AND publish_state = 'queued'
               AND retry_count < 8 AND next_retry_at <= ?2
             ORDER BY created_at,event_id LIMIT ?3",
        )
        .map_err(|_| "command brief spool unavailable")?;
    let rows = statement
        .query_map(params![owner_pubkey, now, limit], |row| {
            Ok(DueSpoolEvent {
                owner_pubkey: row.get(0)?,
                run_id: row.get(1)?,
                event_id: row.get(2)?,
                raw_event: row.get(3)?,
                publish_state: parse_state(&row.get::<_, String>(4)?)?,
                retry_count: row.get(5)?,
                next_retry_at: row.get(6)?,
            })
        })
        .map_err(|_| "command brief spool unavailable")?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|_| "command brief spool unavailable".into())
}

/// Return the latest append-only lifecycle head for one owner/run.
pub fn latest_event_id(
    conn: &Connection,
    owner_pubkey: &str,
    run_id: &str,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT event_id FROM command_brief_spool
         WHERE owner_pubkey=?1 AND run_id=?2
         ORDER BY created_at DESC,rowid DESC LIMIT 1",
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
        || insert.raw_event.len() > 2 * 1024 * 1024
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
