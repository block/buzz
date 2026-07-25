use std::str::FromStr;

use chrono::{NaiveDate, NaiveTime};
use chrono_tz::Tz;
use rusqlite::Connection;
use sha2::{Digest, Sha256};

pub(super) const SCHEMA_VERSION: i64 = 6;

pub(super) const SPOOL_TABLE_SQL: &str = "
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
    append_sequence     INTEGER NOT NULL,
    published_at        INTEGER,
    PRIMARY KEY (owner_pubkey, run_id, event_id),
    UNIQUE (owner_pubkey, event_id),
    UNIQUE (owner_pubkey, run_id, append_sequence)
);";

pub(super) const HEADS_TABLE_SQL: &str = "
CREATE TABLE command_brief_heads (
    owner_pubkey       TEXT NOT NULL,
    run_id             TEXT NOT NULL,
    head_event_id      TEXT NOT NULL,
    head_sequence      INTEGER NOT NULL,
    PRIMARY KEY (owner_pubkey, run_id)
);";

pub(super) const SPOOL_DUE_INDEX_SQL: &str = "
CREATE INDEX command_brief_spool_due
ON command_brief_spool(owner_pubkey, publish_state, next_retry_at, append_sequence);";

pub(super) const SCHEDULE_TABLE_SQL: &str = "
CREATE TABLE command_brief_schedule (
    schedule_id       TEXT PRIMARY KEY,
    classification    TEXT NOT NULL CHECK (classification = 'OFFICIAL'),
    enabled           INTEGER NOT NULL CHECK (enabled IN (0,1)),
    local_time        TEXT NOT NULL,
    timezone          TEXT NOT NULL,
    catch_up_same_day INTEGER NOT NULL CHECK (catch_up_same_day IN (0,1)),
    concurrency       INTEGER NOT NULL CHECK (concurrency IN (1,2)),
    updated_at        INTEGER NOT NULL
);";

pub(super) const CLAIMS_TABLE_SQL: &str = "
CREATE TABLE command_brief_schedule_claims (
    idempotency_key   TEXT PRIMARY KEY,
    schedule_id       TEXT NOT NULL CHECK (schedule_id = 'daily-command-brief'),
    local_date        TEXT NOT NULL CHECK (length(local_date) = 10),
    timezone          TEXT NOT NULL CHECK (length(timezone) BETWEEN 1 AND 128),
    state             TEXT NOT NULL CHECK (state IN ('claimed','deferred','started','completed')),
    deferred_reason   TEXT,
    retry_count       INTEGER NOT NULL DEFAULT 0 CHECK (retry_count BETWEEN 0 AND 8),
    transition_token  TEXT,
    claimed_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    run_id            TEXT NOT NULL CHECK (
        length(run_id) = 74 AND substr(run_id,1,10) = 'scheduled-'
    ),
    CHECK (idempotency_key = schedule_id || ':' || local_date),
    CHECK (
        transition_token IS NULL
        OR (
            length(CAST(transition_token AS BLOB)) BETWEEN 1 AND 256
            AND instr(transition_token,char(0)) = 0
            AND transition_token NOT GLOB '*[^!-~]*'
        )
    ),
    CHECK (
        (state = 'deferred'
         AND deferred_reason IN (
             'identity_locked','model_unavailable',
             'admission_unavailable','local_state_unavailable'
         )
         AND transition_token IS NOT NULL)
        OR
        (state <> 'deferred'
         AND deferred_reason IS NULL)
    ),
    UNIQUE (schedule_id, local_date)
);";

pub(super) const CLAIMS_DEFERRED_INDEX_SQL: &str = "
CREATE INDEX command_brief_schedule_deferred
ON command_brief_schedule_claims(state, retry_count, updated_at);";

pub(super) fn create_current_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
    for sql in [
        SPOOL_TABLE_SQL,
        HEADS_TABLE_SQL,
        SPOOL_DUE_INDEX_SQL,
        SCHEDULE_TABLE_SQL,
        CLAIMS_TABLE_SQL,
        CLAIMS_DEFERRED_INDEX_SQL,
    ] {
        conn.execute_batch(sql)?;
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Column {
    name: String,
    kind: String,
    not_null: i64,
    default: Option<String>,
    primary_key: i64,
    hidden: i64,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Index {
    name: Option<String>,
    unique: i64,
    origin: String,
    partial: i64,
    columns: Vec<String>,
}

/// Verify every production table, index, constraint, version, and semantic row.
pub fn validate_command_brief_store_schema(conn: &Connection) -> Result<(), String> {
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|_| rejected())?;
    let integrity: String = conn
        .pragma_query_value(None, "integrity_check", |row| row.get(0))
        .map_err(|_| rejected())?;
    if version != SCHEMA_VERSION || integrity != "ok" {
        return Err(rejected());
    }

    for (table, sql, columns, indexes) in [
        (
            "command_brief_spool",
            SPOOL_TABLE_SQL,
            spool_columns(),
            vec![
                index(
                    Some("command_brief_spool_due"),
                    0,
                    "c",
                    &[
                        "owner_pubkey",
                        "publish_state",
                        "next_retry_at",
                        "append_sequence",
                    ],
                ),
                index(None, 1, "pk", &["owner_pubkey", "run_id", "event_id"]),
                index(None, 1, "u", &["owner_pubkey", "event_id"]),
                index(None, 1, "u", &["owner_pubkey", "run_id", "append_sequence"]),
            ],
        ),
        (
            "command_brief_heads",
            HEADS_TABLE_SQL,
            columns(&[
                ("owner_pubkey", "TEXT", 1, None, 1),
                ("run_id", "TEXT", 1, None, 2),
                ("head_event_id", "TEXT", 1, None, 0),
                ("head_sequence", "INTEGER", 1, None, 0),
            ]),
            vec![index(None, 1, "pk", &["owner_pubkey", "run_id"])],
        ),
        (
            "command_brief_schedule",
            SCHEDULE_TABLE_SQL,
            columns(&[
                ("schedule_id", "TEXT", 0, None, 1),
                ("classification", "TEXT", 1, None, 0),
                ("enabled", "INTEGER", 1, None, 0),
                ("local_time", "TEXT", 1, None, 0),
                ("timezone", "TEXT", 1, None, 0),
                ("catch_up_same_day", "INTEGER", 1, None, 0),
                ("concurrency", "INTEGER", 1, None, 0),
                ("updated_at", "INTEGER", 1, None, 0),
            ]),
            vec![index(None, 1, "pk", &["schedule_id"])],
        ),
        (
            "command_brief_schedule_claims",
            CLAIMS_TABLE_SQL,
            columns(&[
                ("idempotency_key", "TEXT", 0, None, 1),
                ("schedule_id", "TEXT", 1, None, 0),
                ("local_date", "TEXT", 1, None, 0),
                ("timezone", "TEXT", 1, None, 0),
                ("state", "TEXT", 1, None, 0),
                ("deferred_reason", "TEXT", 0, None, 0),
                ("retry_count", "INTEGER", 1, Some("0"), 0),
                ("transition_token", "TEXT", 0, None, 0),
                ("claimed_at", "INTEGER", 1, None, 0),
                ("updated_at", "INTEGER", 1, None, 0),
                ("run_id", "TEXT", 1, None, 0),
            ]),
            vec![
                index(
                    Some("command_brief_schedule_deferred"),
                    0,
                    "c",
                    &["state", "retry_count", "updated_at"],
                ),
                index(None, 1, "pk", &["idempotency_key"]),
                index(None, 1, "u", &["schedule_id", "local_date"]),
            ],
        ),
    ] {
        if actual_columns(conn, table)? != columns
            || actual_indexes(conn, table)? != sorted(indexes)
            || actual_sql(conn, "table", table)? != canonical_sql(sql)
        {
            return Err(rejected());
        }
    }
    for (name, sql) in [
        ("command_brief_spool_due", SPOOL_DUE_INDEX_SQL),
        ("command_brief_schedule_deferred", CLAIMS_DEFERRED_INDEX_SQL),
    ] {
        if actual_sql(conn, "index", name)? != canonical_sql(sql) {
            return Err(rejected());
        }
    }
    validate_schedule_rows(conn)?;
    validate_claim_rows(conn)
}

fn validate_schedule_rows(conn: &Connection) -> Result<(), String> {
    let mut statement = conn
        .prepare(
            "SELECT schedule_id,classification,enabled,local_time,timezone,
                    catch_up_same_day,concurrency
             FROM command_brief_schedule",
        )
        .map_err(|_| rejected())?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })
        .map_err(|_| rejected())?;
    for row in rows {
        let (id, classification, enabled, local_time, timezone, catch_up, concurrency) =
            row.map_err(|_| rejected())?;
        if id != "daily-command-brief"
            || classification != "OFFICIAL"
            || !matches!(enabled, 0 | 1)
            || NaiveTime::parse_from_str(&local_time, "%H:%M").is_err()
            || local_time.len() != 5
            || Tz::from_str(&timezone).is_err()
            || !matches!(catch_up, 0 | 1)
            || !matches!(concurrency, 1 | 2)
        {
            return Err(rejected());
        }
    }
    Ok(())
}

fn validate_claim_rows(conn: &Connection) -> Result<(), String> {
    let mut statement = conn
        .prepare(
            "SELECT idempotency_key,schedule_id,local_date,timezone,run_id,
                    state,deferred_reason,retry_count,transition_token,claimed_at,updated_at
             FROM command_brief_schedule_claims",
        )
        .map_err(|_| rejected())?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, i64>(9)?,
                row.get::<_, i64>(10)?,
            ))
        })
        .map_err(|_| rejected())?;
    for row in rows {
        let (key, schedule, date, timezone, run_id, state, reason, retry, token, claimed, updated) =
            row.map_err(|_| rejected())?;
        let expected_run = format!("scheduled-{}", hex::encode(Sha256::digest(key.as_bytes())));
        let deferred = state == "deferred";
        if key != format!("{schedule}:{date}")
            || schedule != "daily-command-brief"
            || run_id != expected_run
            || NaiveDate::parse_from_str(&date, "%Y-%m-%d").is_err()
            || Tz::from_str(&timezone).is_err()
            || !matches!(
                state.as_str(),
                "claimed" | "deferred" | "started" | "completed"
            )
            || !(0..=8).contains(&retry)
            || claimed > updated
            || deferred
                != (matches!(
                    reason.as_deref(),
                    Some(
                        "identity_locked"
                            | "model_unavailable"
                            | "admission_unavailable"
                            | "local_state_unavailable"
                    )
                ) && token.as_deref().is_some_and(valid_transition_token))
            || (!deferred
                && (reason.is_some()
                    || token
                        .as_deref()
                        .is_some_and(|value| !valid_transition_token(value))))
        {
            return Err(rejected());
        }
    }
    Ok(())
}

fn actual_sql(conn: &Connection, kind: &str, name: &str) -> Result<String, String> {
    conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type=?1 AND name=?2",
        [kind, name],
        |row| row.get::<_, String>(0),
    )
    .map(|sql| canonical_sql(&sql))
    .map_err(|_| rejected())
}

fn actual_columns(conn: &Connection, table: &str) -> Result<Vec<Column>, String> {
    let mut statement = conn
        .prepare(&format!("PRAGMA table_xinfo({table})"))
        .map_err(|_| rejected())?;
    let columns = statement
        .query_map([], |row| {
            Ok(Column {
                name: row.get(1)?,
                kind: row.get(2)?,
                not_null: row.get(3)?,
                default: row.get(4)?,
                primary_key: row.get(5)?,
                hidden: row.get(6)?,
            })
        })
        .map_err(|_| rejected())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| rejected())?;
    Ok(columns)
}

fn actual_indexes(conn: &Connection, table: &str) -> Result<Vec<Index>, String> {
    let mut statement = conn
        .prepare(&format!("PRAGMA index_list({table})"))
        .map_err(|_| rejected())?;
    let metadata = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(|_| rejected())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| rejected())?;
    let mut indexes = Vec::with_capacity(metadata.len());
    for (name, unique, origin, partial) in metadata {
        let mut columns_statement = conn
            .prepare(&format!("PRAGMA index_info({name})"))
            .map_err(|_| rejected())?;
        let columns = columns_statement
            .query_map([], |row| row.get::<_, String>(2))
            .map_err(|_| rejected())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| rejected())?;
        indexes.push(Index {
            name: (origin == "c").then_some(name),
            unique,
            origin,
            partial,
            columns,
        });
    }
    Ok(sorted(indexes))
}

fn spool_columns() -> Vec<Column> {
    columns(&[
        ("owner_pubkey", "TEXT", 1, None, 1),
        ("run_id", "TEXT", 1, None, 2),
        ("event_id", "TEXT", 1, None, 3),
        ("status", "TEXT", 1, None, 0),
        ("previous_event_id", "TEXT", 0, None, 0),
        ("encrypted_payload", "TEXT", 1, None, 0),
        ("raw_event", "TEXT", 1, None, 0),
        ("publish_state", "TEXT", 1, None, 0),
        ("retry_count", "INTEGER", 1, Some("0"), 0),
        ("next_retry_at", "INTEGER", 1, Some("0"), 0),
        ("last_error_code", "TEXT", 0, None, 0),
        ("created_at", "INTEGER", 1, None, 0),
        ("append_sequence", "INTEGER", 1, None, 0),
        ("published_at", "INTEGER", 0, None, 0),
    ])
}

fn columns(values: &[(&str, &str, i64, Option<&str>, i64)]) -> Vec<Column> {
    values
        .iter()
        .map(|(name, kind, not_null, default, primary_key)| Column {
            name: (*name).to_string(),
            kind: (*kind).to_string(),
            not_null: *not_null,
            default: default.map(str::to_string),
            primary_key: *primary_key,
            hidden: 0,
        })
        .collect()
}

fn index(name: Option<&str>, unique: i64, origin: &str, columns: &[&str]) -> Index {
    Index {
        name: name.map(str::to_string),
        unique,
        origin: origin.to_string(),
        partial: 0,
        columns: columns.iter().map(|column| (*column).to_string()).collect(),
    }
}

fn sorted(mut indexes: Vec<Index>) -> Vec<Index> {
    indexes.sort();
    indexes
}

fn canonical_sql(sql: &str) -> String {
    sql.chars()
        .filter(|character| {
            !character.is_ascii_whitespace()
                && !matches!(character, '"' | '`' | '[' | ']')
                && *character != ';'
        })
        .flat_map(char::to_lowercase)
        .collect()
}

fn valid_transition_token(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && value.bytes().all(|byte| byte.is_ascii_graphic())
}

fn rejected() -> String {
    "command brief store schema rejected".to_string()
}
