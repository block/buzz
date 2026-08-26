//! Durable execution log for broker requests.
//!
//! One row per `(relay_scope, owner, requester, capability, request_id)`. The
//! row is inserted in state `executing` **before the first side effect**, so a
//! request that crashes mid-flight leaves evidence behind rather than looking
//! like it never ran.
//!
//! # What this buys, and what it does not
//!
//! This log delivers **at-most-once execution plus `indeterminate`**. It does
//! not deliver exactly-once or durable completion. A crash between a mutation
//! and the recording of its result is not recoverable from here — the log can
//! prove that execution *started*, never how far it got. Reconciling partial
//! side effects is the owning capability's job, because only it knows what its
//! phases were.
//!
//! WAL + `busy_timeout=5000` matches `managed_agents/retention.rs` and
//! `archive/store.rs`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use super::pipeline::{BoxFuture, ExecutionLog};

/// Terminal or in-flight state of a broker request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionState {
    /// A handler was dispatched and has not reported back.
    Executing,
    /// The handler completed successfully.
    Succeeded,
    /// The handler failed without leaving side effects.
    Failed,
    /// Whether side effects took hold is unknown.
    Indeterminate,
}

impl ExecutionState {
    /// Stable database encoding.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Executing => "executing",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Indeterminate => "indeterminate",
        }
    }

    /// Parse the database encoding.
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "executing" => Ok(Self::Executing),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "indeterminate" => Ok(Self::Indeterminate),
            other => Err(format!("unknown broker execution state \"{other}\"")),
        }
    }

    /// Whether this state is final.
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Executing)
    }
}

/// Identity of one broker request, as derived by the host.
///
/// Owner, requester, and relay scope come from the verified frame — never from
/// the request body — so a caller cannot address another owner's execution log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionKey {
    /// Relay the request arrived on, normalized.
    pub relay_scope: String,
    /// Owner whose credentials would be used.
    pub owner_pubkey: String,
    /// Verified signer of the request frame.
    pub requester_pubkey: String,
    /// Capability wire name.
    pub capability: String,
    /// Caller-chosen idempotency key.
    pub request_id: String,
}

/// A recorded execution row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionRecord {
    /// Current state.
    pub state: ExecutionState,
    /// Digest of the request that claimed this key.
    pub request_digest: String,
    /// Capability contract version that claimed this key.
    pub capability_version: u16,
    /// Serialized terminal result, absent while `executing`.
    pub result_json: Option<String>,
}

/// What a claim attempt established.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimOutcome {
    /// This caller owns the execution and must proceed.
    Claimed,
    /// A terminal result already exists; replay it without re-executing.
    Replay(ExecutionRecord),
    /// A previous attempt started and never finished.
    ///
    /// The row has been moved to `indeterminate` and must not be re-executed:
    /// side effects may already exist.
    Interrupted(ExecutionRecord),
    /// The key was claimed by a request with different content.
    DigestConflict {
        /// Digest recorded by the first request.
        expected: String,
    },
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS broker_executions (
    relay_scope       TEXT NOT NULL,
    owner_pubkey      TEXT NOT NULL,
    requester_pubkey  TEXT NOT NULL,
    capability        TEXT NOT NULL,
    request_id        TEXT NOT NULL,
    request_digest    TEXT NOT NULL,
    capability_version INTEGER NOT NULL,
    state             TEXT NOT NULL,
    result_json       TEXT,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    PRIMARY KEY (relay_scope, owner_pubkey, requester_pubkey, capability, request_id)
);
";

/// Relay-URL form that identifies a broker scope.
///
/// Mirrors `managed_agents::retention`: equivalent workspace URLs (surrounding
/// space, trailing slash) must resolve to one scope, so "same relay" can never
/// disagree with "same database".
pub fn normalized_relay_scope(relay_url: &str) -> &str {
    relay_url.trim().trim_end_matches('/')
}

/// Resolve the broker database path for a relay + owner pair.
///
/// The normalized scope is hashed so relay URLs never become path components.
pub fn scoped_broker_db_path(base_dir: &Path, relay_url: &str, owner_pubkey: &str) -> PathBuf {
    let normalized_relay = normalized_relay_scope(relay_url);
    let mut hasher = Sha256::new();
    hasher.update(owner_pubkey.trim().to_ascii_lowercase().as_bytes());
    hasher.update(b"\0");
    hasher.update(normalized_relay.as_bytes());
    let scope_id = hex::encode(hasher.finalize());
    base_dir.join("broker").join(format!("{scope_id}.db"))
}

/// Open (or create) the broker execution database.
pub fn open_broker_db(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("failed to create broker dir: {e}"))?;
    }

    let conn = Connection::open(path).map_err(|e| format!("failed to open broker db: {e}"))?;

    conn.pragma_update(None, "busy_timeout", 5000)
        .map_err(|e| format!("failed to set busy_timeout: {e}"))?;
    set_wal_mode(&conn)?;

    conn.execute_batch(SCHEMA)
        .map_err(|e| format!("failed to initialize broker schema: {e}"))?;

    Ok(conn)
}

fn set_wal_mode(conn: &Connection) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match conn.pragma_update(None, "journal_mode", "WAL") {
            Ok(()) => return Ok(()),
            Err(error) if sqlite_is_busy(&error) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(error) => return Err(format!("failed to set WAL mode: {error}")),
        }
    }
}

fn sqlite_is_busy(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(inner, _)
            if matches!(
                inner.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            )
    )
}

/// Claim `key` for execution, or report why it cannot be claimed.
///
/// The insert and the read happen in one `IMMEDIATE` transaction, so two
/// concurrent frames carrying the same request cannot both believe they claimed
/// it. This is the only function that may transition a row into `executing`,
/// and it never returns [`ClaimOutcome::Claimed`] for a key that already has a
/// row.
///
/// An existing `executing` row is treated as interrupted: it is moved to
/// `indeterminate` and returned. Re-executing it would risk duplicating side
/// effects that the log cannot see.
pub fn claim(
    conn: &mut Connection,
    key: &ExecutionKey,
    request_digest: &str,
    capability_version: u16,
    now: i64,
) -> Result<ClaimOutcome, String> {
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| format!("failed to begin broker claim: {e}"))?;

    let existing = tx
        .query_row(
            "SELECT state, request_digest, capability_version, result_json
               FROM broker_executions
              WHERE relay_scope = ?1 AND owner_pubkey = ?2 AND requester_pubkey = ?3
                AND capability = ?4 AND request_id = ?5",
            params![
                key.relay_scope,
                key.owner_pubkey,
                key.requester_pubkey,
                key.capability,
                key.request_id
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|e| format!("failed to read broker execution: {e}"))?;

    let outcome = match existing {
        None => {
            tx.execute(
                "INSERT INTO broker_executions (
                     relay_scope, owner_pubkey, requester_pubkey, capability, request_id,
                     request_digest, capability_version, state, result_json,
                     created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9, ?9)",
                params![
                    key.relay_scope,
                    key.owner_pubkey,
                    key.requester_pubkey,
                    key.capability,
                    key.request_id,
                    request_digest,
                    i64::from(capability_version),
                    ExecutionState::Executing.as_str(),
                    now
                ],
            )
            .map_err(|e| format!("failed to claim broker execution: {e}"))?;
            ClaimOutcome::Claimed
        }
        Some((state, digest, version, result_json)) => {
            // A different payload under the same id is a caller bug, and the
            // recorded outcome does not describe it. Checked before state so a
            // conflicting retry can never be answered with someone else's result.
            if digest != request_digest {
                ClaimOutcome::DigestConflict { expected: digest }
            } else {
                let state = ExecutionState::parse(&state)?;
                let version = u16::try_from(version)
                    .map_err(|_| format!("stored capability_version {version} out of range"))?;
                let record = ExecutionRecord {
                    state,
                    request_digest: digest,
                    capability_version: version,
                    result_json,
                };
                if state.is_terminal() {
                    ClaimOutcome::Replay(record)
                } else {
                    tx.execute(
                        "UPDATE broker_executions SET state = ?1, updated_at = ?2
                          WHERE relay_scope = ?3 AND owner_pubkey = ?4
                            AND requester_pubkey = ?5 AND capability = ?6
                            AND request_id = ?7",
                        params![
                            ExecutionState::Indeterminate.as_str(),
                            now,
                            key.relay_scope,
                            key.owner_pubkey,
                            key.requester_pubkey,
                            key.capability,
                            key.request_id
                        ],
                    )
                    .map_err(|e| format!("failed to mark broker execution interrupted: {e}"))?;
                    ClaimOutcome::Interrupted(ExecutionRecord {
                        state: ExecutionState::Indeterminate,
                        ..record
                    })
                }
            }
        }
    };

    tx.commit()
        .map_err(|e| format!("failed to commit broker claim: {e}"))?;
    Ok(outcome)
}

/// Record the terminal state and result for a claimed key.
///
/// Rejects a non-terminal state: only [`claim`] may write `executing`.
pub fn complete(
    conn: &Connection,
    key: &ExecutionKey,
    state: ExecutionState,
    result_json: &str,
    now: i64,
) -> Result<(), String> {
    if !state.is_terminal() {
        return Err(format!(
            "cannot complete a broker execution as \"{}\"",
            state.as_str()
        ));
    }
    let changed = conn
        .execute(
            "UPDATE broker_executions
                SET state = ?1, result_json = ?2, updated_at = ?3
              WHERE relay_scope = ?4 AND owner_pubkey = ?5 AND requester_pubkey = ?6
                AND capability = ?7 AND request_id = ?8",
            params![
                state.as_str(),
                result_json,
                now,
                key.relay_scope,
                key.owner_pubkey,
                key.requester_pubkey,
                key.capability,
                key.request_id
            ],
        )
        .map_err(|e| format!("failed to complete broker execution: {e}"))?;
    if changed == 0 {
        return Err("broker execution row is missing; refusing to record a result".into());
    }
    Ok(())
}

/// Open an in-memory execution log with the real schema.
///
/// For tests that need the real claim/complete behavior without a temp file.
#[cfg(test)]
pub fn open_in_memory() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory sqlite");
    conn.execute_batch(SCHEMA).expect("broker schema");
    conn
}

/// In-memory [`ExecutionLog`] running the real [`claim`] / [`complete`] logic.
///
/// Tests get the production idempotency behavior without touching the
/// filesystem. `Connection` is `Send` but not `Sync`, so the mutex is what makes
/// it shareable behind the `Send + Sync` trait — and holding the lock for the
/// whole call serializes claims the same way the file-backed `IMMEDIATE`
/// transaction does.
#[cfg(test)]
pub struct MemoryExecutionLog(std::sync::Mutex<Connection>);

#[cfg(test)]
impl MemoryExecutionLog {
    /// A fresh, empty log.
    pub fn new() -> Self {
        Self(std::sync::Mutex::new(open_in_memory()))
    }

    /// Run `body` against the underlying connection.
    ///
    /// Lets a test plant a row directly — simulating a process that claimed a
    /// key and died — without exposing the connection itself.
    pub fn with_conn<T>(&self, body: impl FnOnce(&mut Connection) -> T) -> T {
        body(&mut self.0.lock().expect("broker test log poisoned"))
    }
}

#[cfg(test)]
impl ExecutionLog for MemoryExecutionLog {
    fn claim(
        &self,
        key: ExecutionKey,
        request_digest: String,
        capability_version: u16,
        now: i64,
    ) -> BoxFuture<'_, Result<ClaimOutcome, String>> {
        Box::pin(async move {
            self.with_conn(|conn| claim(conn, &key, &request_digest, capability_version, now))
        })
    }

    fn complete(
        &self,
        key: ExecutionKey,
        state: ExecutionState,
        result_json: String,
        now: i64,
    ) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(
            async move { self.with_conn(|conn| complete(conn, &key, state, &result_json, now)) },
        )
    }
}

/// File-backed [`ExecutionLog`] that keeps every `!Sync` connection inside one
/// blocking task.
///
/// The pipeline is async, so it can never hold a `rusqlite::Connection` across
/// an `.await`. This adapter is where that constraint is discharged: each call
/// opens its own connection on the blocking pool, finishes its transaction, and
/// drops it before the future resolves. Cross-call exclusion is SQLite's, not
/// this type's — [`claim`] does its read and insert inside one `IMMEDIATE`
/// transaction, which is what makes two concurrent claims safe even though they
/// use different connections.
pub struct SqliteExecutionLog {
    path: PathBuf,
}

impl SqliteExecutionLog {
    /// Log stored at `path`. The file and its parent are created on first use.
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl ExecutionLog for SqliteExecutionLog {
    fn claim(
        &self,
        key: ExecutionKey,
        request_digest: String,
        capability_version: u16,
        now: i64,
    ) -> BoxFuture<'_, Result<ClaimOutcome, String>> {
        let path = self.path.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let mut conn = open_broker_db(&path)?;
                claim(&mut conn, &key, &request_digest, capability_version, now)
            })
            .await
            .map_err(|error| format!("broker claim task failed: {error}"))?
        })
    }

    fn complete(
        &self,
        key: ExecutionKey,
        state: ExecutionState,
        result_json: String,
        now: i64,
    ) -> BoxFuture<'_, Result<(), String>> {
        let path = self.path.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let conn = open_broker_db(&path)?;
                complete(&conn, &key, state, &result_json, now)
            })
            .await
            .map_err(|error| format!("broker complete task failed: {error}"))?
        })
    }
}

/// Read one execution row, if it exists.
///
/// Test-only: the pipeline never reads a record outside [`claim`], which returns
/// what it found in the same transaction that decided whether to claim. A
/// separate read would be a second, racier way to ask the same question.
#[cfg(test)]
pub fn get(conn: &Connection, key: &ExecutionKey) -> Result<Option<ExecutionRecord>, String> {
    conn.query_row(
        "SELECT state, request_digest, capability_version, result_json
           FROM broker_executions
          WHERE relay_scope = ?1 AND owner_pubkey = ?2 AND requester_pubkey = ?3
            AND capability = ?4 AND request_id = ?5",
        params![
            key.relay_scope,
            key.owner_pubkey,
            key.requester_pubkey,
            key.capability,
            key.request_id
        ],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        },
    )
    .optional()
    .map_err(|e| format!("failed to read broker execution: {e}"))?
    .map(|(state, request_digest, version, result_json)| {
        Ok(ExecutionRecord {
            state: ExecutionState::parse(&state)?,
            request_digest,
            capability_version: u16::try_from(version)
                .map_err(|_| format!("stored capability_version {version} out of range"))?,
            result_json,
        })
    })
    .transpose()
}

#[cfg(test)]
mod tests;
