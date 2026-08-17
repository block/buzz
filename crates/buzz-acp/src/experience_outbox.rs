//! Crash-safe local queue for signed experience events and their projections.

use std::{path::Path, sync::Mutex};

use buzz_core::agent_memory_canonical::canonical_json_bytes;
use nostr::Event;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use sha2::{Digest, Sha256};

const MAX_PROJECTION_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OutboxState {
    Pending,
    Published,
    Projected,
}

impl OutboxState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Published => "published",
            Self::Projected => "projected",
        }
    }

    fn parse(value: &str) -> Result<Self, ExperienceOutboxError> {
        match value {
            "pending" => Ok(Self::Pending),
            "published" => Ok(Self::Published),
            "projected" => Ok(Self::Projected),
            _ => Err(ExperienceOutboxError::InvalidState),
        }
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) struct OutboxEntry {
    pub record_id: String,
    pub signed_event: Event,
    pub projection_payload: Value,
    pub projection_payload_hash: String,
    pub state: OutboxState,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct OutboxHealth {
    pub pending: u64,
    pub published: u64,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ExperienceOutboxError {
    #[error("experience outbox database failed")]
    Sqlite(#[from] rusqlite::Error),
    #[error("experience outbox serialization failed")]
    Serialization(#[from] serde_json::Error),
    #[error("experience outbox canonicalization failed")]
    Canonicalization,
    #[error("experience outbox lock failed")]
    Lock,
    #[error("experience outbox entry conflicts with existing record")]
    Conflict,
    #[error("experience outbox entry was not found")]
    NotFound,
    #[error("experience outbox contains an invalid state")]
    InvalidState,
}

pub(crate) struct ExperienceOutbox {
    connection: Mutex<Connection>,
}

impl ExperienceOutbox {
    pub(crate) fn open(path: &Path) -> Result<Self, ExperienceOutboxError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| ExperienceOutboxError::Lock)?;
        }
        let connection = Connection::open(path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS experience_outbox (
                record_id TEXT PRIMARY KEY NOT NULL,
                signed_event_json TEXT NOT NULL,
                projection_payload_json TEXT NOT NULL,
                projection_payload_hash TEXT NOT NULL,
                state TEXT NOT NULL CHECK (state IN ('pending', 'published', 'projected')),
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Enqueue once. Identical recovery is idempotent; divergent reuse is rejected.
    pub(crate) fn enqueue(
        &self,
        record_id: &str,
        event: &Event,
        projection_payload: &Value,
    ) -> Result<bool, ExperienceOutboxError> {
        let signed_event_json = serde_json::to_string(event)?;
        let projection_bytes = canonical_json_bytes(projection_payload, MAX_PROJECTION_BYTES)
            .map_err(|_| ExperienceOutboxError::Canonicalization)?;
        let projection_payload_json = String::from_utf8(projection_bytes.clone())
            .map_err(|_| ExperienceOutboxError::Canonicalization)?;
        let projection_payload_hash = hex::encode(Sha256::digest(&projection_bytes));
        let connection = self
            .connection
            .lock()
            .map_err(|_| ExperienceOutboxError::Lock)?;
        let inserted = connection.execute(
            "INSERT OR IGNORE INTO experience_outbox
             (record_id, signed_event_json, projection_payload_json, projection_payload_hash, state)
             VALUES (?1, ?2, ?3, ?4, 'pending')",
            params![
                record_id,
                signed_event_json,
                projection_payload_json,
                projection_payload_hash
            ],
        )?;
        if inserted == 1 {
            return Ok(true);
        }

        let existing: Option<(String, String)> = connection
            .query_row(
                "SELECT signed_event_json, projection_payload_hash
                 FROM experience_outbox WHERE record_id = ?1",
                [record_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match existing {
            Some((existing_event, existing_hash))
                if existing_event == signed_event_json
                    && existing_hash == projection_payload_hash =>
            {
                Ok(false)
            }
            Some(_) => Err(ExperienceOutboxError::Conflict),
            None => Err(ExperienceOutboxError::NotFound),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn get(&self, record_id: &str) -> Result<OutboxEntry, ExperienceOutboxError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| ExperienceOutboxError::Lock)?;
        Self::query_entry(&connection, record_id)?.ok_or(ExperienceOutboxError::NotFound)
    }

    pub(crate) fn mark_published(&self, record_id: &str) -> Result<(), ExperienceOutboxError> {
        self.advance(record_id, OutboxState::Published)
    }

    #[allow(dead_code)]
    pub(crate) fn mark_projected(&self, record_id: &str) -> Result<(), ExperienceOutboxError> {
        self.advance(record_id, OutboxState::Projected)
    }

    pub(crate) fn ready_for_publish(&self) -> Result<Vec<OutboxEntry>, ExperienceOutboxError> {
        self.entries_in_state(OutboxState::Pending)
    }

    #[allow(dead_code)]
    pub(crate) fn ready_for_projection(&self) -> Result<Vec<OutboxEntry>, ExperienceOutboxError> {
        self.entries_in_state(OutboxState::Published)
    }

    pub(crate) fn health(&self) -> Result<OutboxHealth, ExperienceOutboxError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| ExperienceOutboxError::Lock)?;
        let pending = connection.query_row(
            "SELECT COUNT(*) FROM experience_outbox WHERE state = 'pending'",
            [],
            |row| row.get(0),
        )?;
        let published = connection.query_row(
            "SELECT COUNT(*) FROM experience_outbox WHERE state = 'published'",
            [],
            |row| row.get(0),
        )?;
        Ok(OutboxHealth { pending, published })
    }

    fn advance(&self, record_id: &str, target: OutboxState) -> Result<(), ExperienceOutboxError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| ExperienceOutboxError::Lock)?;
        let current: Option<String> = connection
            .query_row(
                "SELECT state FROM experience_outbox WHERE record_id = ?1",
                [record_id],
                |row| row.get(0),
            )
            .optional()?;
        let current = current.ok_or(ExperienceOutboxError::NotFound)?;
        let current = OutboxState::parse(&current)?;
        let allowed = match (current, target) {
            (OutboxState::Pending, OutboxState::Published)
            | (OutboxState::Published, OutboxState::Projected) => true,
            (state, requested) if state == requested => false,
            _ => return Err(ExperienceOutboxError::InvalidState),
        };
        if allowed {
            connection.execute(
                "UPDATE experience_outbox SET state = ?1 WHERE record_id = ?2",
                params![target.as_str(), record_id],
            )?;
        }
        Ok(())
    }

    fn entries_in_state(
        &self,
        state: OutboxState,
    ) -> Result<Vec<OutboxEntry>, ExperienceOutboxError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| ExperienceOutboxError::Lock)?;
        let mut statement = connection.prepare(
            "SELECT record_id, signed_event_json, projection_payload_json,
                    projection_payload_hash, state
             FROM experience_outbox WHERE state = ?1 ORDER BY created_at, record_id",
        )?;
        let rows = statement.query_map([state.as_str()], Self::row_to_entry)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    #[allow(dead_code)]
    fn query_entry(
        connection: &Connection,
        record_id: &str,
    ) -> Result<Option<OutboxEntry>, ExperienceOutboxError> {
        connection
            .query_row(
                "SELECT record_id, signed_event_json, projection_payload_json,
                        projection_payload_hash, state
                 FROM experience_outbox WHERE record_id = ?1",
                [record_id],
                Self::row_to_entry,
            )
            .optional()
            .map_err(Into::into)
    }

    fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<OutboxEntry> {
        let signed_event_json: String = row.get(1)?;
        let projection_payload_json: String = row.get(2)?;
        let state: String = row.get(4)?;
        let signed_event = serde_json::from_str(&signed_event_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                signed_event_json.len(),
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        let projection_payload =
            serde_json::from_str(&projection_payload_json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    projection_payload_json.len(),
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
        let state = OutboxState::parse(&state).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                state.len(),
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        Ok(OutboxEntry {
            record_id: row.get(0)?,
            signed_event,
            projection_payload,
            projection_payload_hash: row.get(3)?,
            state,
        })
    }
}
