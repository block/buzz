use std::{collections::HashMap, path::Path, sync::Mutex};

use buzz_core::agent_skill::SkillVersionV1;
use nostr::Event;
use rusqlite::{params, Connection, OptionalExtension};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PublicationKind {
    Version,
    Pointer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PublicationState {
    Pending,
    VersionPublished,
    PointerPublished,
    Materialized,
}

impl PublicationState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::VersionPublished => "version_published",
            Self::PointerPublished => "pointer_published",
            Self::Materialized => "materialized",
        }
    }

    fn parse(value: &str) -> Result<Self, RegistryError> {
        match value {
            "pending" => Ok(Self::Pending),
            "version_published" => Ok(Self::VersionPublished),
            "pointer_published" => Ok(Self::PointerPublished),
            "materialized" => Ok(Self::Materialized),
            _ => Err(RegistryError::InvalidState),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PublicationWork {
    pub operation_id: String,
    pub skill_id: String,
    pub version_id: String,
    pub kind: PublicationKind,
    pub event: Event,
}

#[derive(Clone, Debug)]
pub(super) struct Observation {
    pub experience_id: String,
    pub occurred_at: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum InsertObservation {
    Inserted,
    Duplicate,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum RegistryError {
    #[error("skill registry database failed")]
    Sqlite(#[from] rusqlite::Error),
    #[error("skill registry serialization failed")]
    Serialization(#[from] serde_json::Error),
    #[error("skill registry lock failed")]
    Lock,
    #[error("skill registry entry conflicts with existing bytes")]
    Conflict,
    #[error("skill registry entry was not found")]
    NotFound,
    #[error("skill registry state transition is invalid")]
    InvalidState,
}

pub(crate) struct SkillRegistry {
    connection: Mutex<Connection>,
}

impl SkillRegistry {
    pub(crate) fn open(path: &Path) -> Result<Self, RegistryError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| RegistryError::Lock)?;
        }
        let connection = Connection::open(path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS skill_observations (
                experience_id TEXT PRIMARY KEY NOT NULL,
                task_hash TEXT NOT NULL,
                normalized_task TEXT NOT NULL,
                outcome TEXT NOT NULL CHECK (outcome IN ('succeeded', 'failed')),
                occurred_at TEXT NOT NULL,
                active_version_id TEXT,
                consumed INTEGER NOT NULL DEFAULT 0 CHECK (consumed IN (0, 1))
            );
            CREATE INDEX IF NOT EXISTS skill_observations_match
              ON skill_observations(task_hash, outcome, active_version_id, consumed);
            CREATE TABLE IF NOT EXISTS skill_versions (
                version_id TEXT PRIMARY KEY NOT NULL,
                skill_id TEXT NOT NULL,
                parent_version_id TEXT,
                payload_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS active_skills (
                skill_id TEXT PRIMARY KEY NOT NULL,
                active_version_id TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS skill_evaluations (
                evaluation_id TEXT PRIMARY KEY NOT NULL,
                version_id TEXT NOT NULL,
                passed INTEGER NOT NULL CHECK (passed IN (0, 1)),
                check_ids_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS skill_publication_outbox (
                operation_id TEXT PRIMARY KEY NOT NULL,
                skill_id TEXT NOT NULL,
                target_version_id TEXT NOT NULL,
                version_event_json TEXT,
                pointer_event_json TEXT NOT NULL,
                state TEXT NOT NULL CHECK (state IN (
                    'pending', 'version_published', 'pointer_published', 'materialized'
                )),
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub(super) fn insert_observation(
        &self,
        experience_id: &str,
        task_hash: &str,
        normalized_task: &str,
        outcome: &str,
        occurred_at: &str,
        active_version_id: Option<&str>,
    ) -> Result<InsertObservation, RegistryError> {
        let connection = self.connection.lock().map_err(|_| RegistryError::Lock)?;
        let inserted = connection.execute(
            "INSERT OR IGNORE INTO skill_observations
             (experience_id, task_hash, normalized_task, outcome, occurred_at, active_version_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                experience_id,
                task_hash,
                normalized_task,
                outcome,
                occurred_at,
                active_version_id
            ],
        )?;
        if inserted == 1 {
            return Ok(InsertObservation::Inserted);
        }
        let existing: Option<(String, String, String, String, Option<String>)> = connection
            .query_row(
                "SELECT task_hash, normalized_task, outcome, occurred_at, active_version_id
                 FROM skill_observations WHERE experience_id = ?1",
                [experience_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;
        match existing {
            Some(existing)
                if existing
                    == (
                        task_hash.to_string(),
                        normalized_task.to_string(),
                        outcome.to_string(),
                        occurred_at.to_string(),
                        active_version_id.map(ToOwned::to_owned),
                    ) =>
            {
                Ok(InsertObservation::Duplicate)
            }
            Some(_) => Err(RegistryError::Conflict),
            None => Err(RegistryError::NotFound),
        }
    }

    pub(super) fn unconsumed_successes(
        &self,
        task_hash: &str,
    ) -> Result<Vec<Observation>, RegistryError> {
        self.observations(
            "task_hash = ?1 AND outcome = 'succeeded' AND consumed = 0",
            task_hash,
        )
    }

    pub(super) fn unconsumed_failures(
        &self,
        task_hash: &str,
        active_version_id: &str,
    ) -> Result<Vec<Observation>, RegistryError> {
        let connection = self.connection.lock().map_err(|_| RegistryError::Lock)?;
        let mut statement = connection.prepare(
            "SELECT experience_id, occurred_at FROM skill_observations
             WHERE task_hash = ?1 AND outcome = 'failed' AND active_version_id = ?2
               AND consumed = 0
             ORDER BY rowid LIMIT 2",
        )?;
        let rows = statement.query_map(params![task_hash, active_version_id], |row| {
            Ok(Observation {
                experience_id: row.get(0)?,
                occurred_at: row.get(1)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    fn observations(
        &self,
        predicate: &str,
        value: &str,
    ) -> Result<Vec<Observation>, RegistryError> {
        let connection = self.connection.lock().map_err(|_| RegistryError::Lock)?;
        let sql = format!(
            "SELECT experience_id, occurred_at FROM skill_observations
             WHERE {predicate} ORDER BY rowid LIMIT 2"
        );
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map([value], |row| {
            Ok(Observation {
                experience_id: row.get(0)?,
                occurred_at: row.get(1)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub(super) fn consume_observations(&self, ids: &[String]) -> Result<(), RegistryError> {
        let mut connection = self.connection.lock().map_err(|_| RegistryError::Lock)?;
        let transaction = connection.transaction()?;
        for id in ids {
            transaction.execute(
                "UPDATE skill_observations SET consumed = 1 WHERE experience_id = ?1",
                [id],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub(super) fn queue_promotion(
        &self,
        version: &SkillVersionV1,
        evaluation_id: &str,
        check_ids: &[String],
        version_event: &Event,
        pointer_event: &Event,
    ) -> Result<(), RegistryError> {
        let payload_json = serde_json::to_string(version)?;
        let version_event_json = serde_json::to_string(version_event)?;
        let pointer_event_json = serde_json::to_string(pointer_event)?;
        let check_ids_json = serde_json::to_string(check_ids)?;
        let mut connection = self.connection.lock().map_err(|_| RegistryError::Lock)?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT OR IGNORE INTO skill_versions
             (version_id, skill_id, parent_version_id, payload_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                version.version_id,
                version.skill_id,
                version.parent_version_id,
                payload_json
            ],
        )?;
        transaction.execute(
            "INSERT OR IGNORE INTO skill_evaluations
             (evaluation_id, version_id, passed, check_ids_json)
             VALUES (?1, ?2, 1, ?3)",
            params![evaluation_id, version.version_id, check_ids_json],
        )?;
        let inserted = transaction.execute(
            "INSERT OR IGNORE INTO skill_publication_outbox
             (operation_id, skill_id, target_version_id, version_event_json,
              pointer_event_json, state)
             VALUES (?1, ?2, ?3, ?4, ?5, 'pending')",
            params![
                version.version_id,
                version.skill_id,
                version.version_id,
                version_event_json,
                pointer_event_json
            ],
        )?;
        if inserted == 0 {
            let existing: String = transaction.query_row(
                "SELECT pointer_event_json FROM skill_publication_outbox WHERE operation_id = ?1",
                [&version.version_id],
                |row| row.get(0),
            )?;
            if existing != serde_json::to_string(pointer_event)? {
                return Err(RegistryError::Conflict);
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub(super) fn queue_rollback(
        &self,
        operation_id: &str,
        skill_id: &str,
        target_version_id: &str,
        pointer_event: &Event,
    ) -> Result<(), RegistryError> {
        let pointer_event_json = serde_json::to_string(pointer_event)?;
        let connection = self.connection.lock().map_err(|_| RegistryError::Lock)?;
        let inserted = connection.execute(
            "INSERT OR IGNORE INTO skill_publication_outbox
             (operation_id, skill_id, target_version_id, version_event_json,
              pointer_event_json, state)
             VALUES (?1, ?2, ?3, NULL, ?4, 'version_published')",
            params![
                operation_id,
                skill_id,
                target_version_id,
                pointer_event_json
            ],
        )?;
        if inserted == 0 {
            let existing: String = connection.query_row(
                "SELECT pointer_event_json FROM skill_publication_outbox WHERE operation_id = ?1",
                [operation_id],
                |row| row.get(0),
            )?;
            if existing != serde_json::to_string(pointer_event)? {
                return Err(RegistryError::Conflict);
            }
        }
        Ok(())
    }

    pub(crate) fn ready_for_publish(&self) -> Result<Vec<PublicationWork>, RegistryError> {
        let connection = self.connection.lock().map_err(|_| RegistryError::Lock)?;
        let mut statement = connection.prepare(
            "SELECT operation_id, skill_id, target_version_id, version_event_json,
                    pointer_event_json, state
             FROM skill_publication_outbox
             WHERE state IN ('pending', 'version_published')
             ORDER BY created_at, operation_id",
        )?;
        let rows = statement.query_map([], |row| {
            let state: String = row.get(5)?;
            let (kind, json): (PublicationKind, String) = if state == "pending" {
                (PublicationKind::Version, row.get(3)?)
            } else {
                (PublicationKind::Pointer, row.get(4)?)
            };
            let event = serde_json::from_str(&json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    json.len(),
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            Ok(PublicationWork {
                operation_id: row.get(0)?,
                skill_id: row.get(1)?,
                version_id: row.get(2)?,
                kind,
                event,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub(super) fn has_inflight_for_skill(&self, skill_id: &str) -> Result<bool, RegistryError> {
        let connection = self.connection.lock().map_err(|_| RegistryError::Lock)?;
        let count: u64 = connection.query_row(
            "SELECT COUNT(*) FROM skill_publication_outbox
             WHERE skill_id = ?1 AND state IN ('pending', 'version_published', 'pointer_published')",
            [skill_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    pub(crate) fn mark_version_published(&self, operation_id: &str) -> Result<(), RegistryError> {
        self.advance(operation_id, PublicationState::VersionPublished)
    }

    pub(crate) fn mark_pointer_published(&self, operation_id: &str) -> Result<(), RegistryError> {
        let mut connection = self.connection.lock().map_err(|_| RegistryError::Lock)?;
        let transaction = connection.transaction()?;
        let row: Option<(String, String, String)> = transaction
            .query_row(
                "SELECT skill_id, target_version_id, state FROM skill_publication_outbox
                 WHERE operation_id = ?1",
                [operation_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let (skill_id, version_id, state) = row.ok_or(RegistryError::NotFound)?;
        match PublicationState::parse(&state)? {
            PublicationState::VersionPublished => {
                transaction.execute(
                    "UPDATE skill_publication_outbox SET state = 'pointer_published'
                     WHERE operation_id = ?1",
                    [operation_id],
                )?;
                transaction.execute(
                    "INSERT INTO active_skills (skill_id, active_version_id) VALUES (?1, ?2)
                     ON CONFLICT(skill_id) DO UPDATE SET active_version_id = excluded.active_version_id",
                    params![skill_id, version_id],
                )?;
            }
            PublicationState::PointerPublished => {}
            _ => return Err(RegistryError::InvalidState),
        }
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn mark_materialized(&self, operation_id: &str) -> Result<(), RegistryError> {
        self.advance(operation_id, PublicationState::Materialized)
    }

    fn advance(&self, operation_id: &str, target: PublicationState) -> Result<(), RegistryError> {
        let connection = self.connection.lock().map_err(|_| RegistryError::Lock)?;
        let state: Option<String> = connection
            .query_row(
                "SELECT state FROM skill_publication_outbox WHERE operation_id = ?1",
                [operation_id],
                |row| row.get(0),
            )
            .optional()?;
        let current = PublicationState::parse(&state.ok_or(RegistryError::NotFound)?)?;
        let allowed = matches!(
            (current, target),
            (
                PublicationState::Pending,
                PublicationState::VersionPublished
            ) | (
                PublicationState::PointerPublished,
                PublicationState::Materialized
            )
        );
        if current == target {
            return Ok(());
        }
        if !allowed {
            return Err(RegistryError::InvalidState);
        }
        connection.execute(
            "UPDATE skill_publication_outbox SET state = ?1 WHERE operation_id = ?2",
            params![target.as_str(), operation_id],
        )?;
        Ok(())
    }

    pub(crate) fn active_version(&self, skill_id: &str) -> Result<Option<String>, RegistryError> {
        let connection = self.connection.lock().map_err(|_| RegistryError::Lock)?;
        connection
            .query_row(
                "SELECT active_version_id FROM active_skills WHERE skill_id = ?1",
                [skill_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub(crate) fn version(
        &self,
        version_id: &str,
    ) -> Result<Option<SkillVersionV1>, RegistryError> {
        let connection = self.connection.lock().map_err(|_| RegistryError::Lock)?;
        let json: Option<String> = connection
            .query_row(
                "SELECT payload_json FROM skill_versions WHERE version_id = ?1",
                [version_id],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|json| serde_json::from_str(&json).map_err(Into::into))
            .transpose()
    }

    pub(crate) fn publication_state(
        &self,
        operation_id: &str,
    ) -> Result<Option<PublicationState>, RegistryError> {
        let connection = self.connection.lock().map_err(|_| RegistryError::Lock)?;
        let state: Option<String> = connection
            .query_row(
                "SELECT state FROM skill_publication_outbox WHERE operation_id = ?1",
                [operation_id],
                |row| row.get(0),
            )
            .optional()?;
        state
            .map(|state| PublicationState::parse(&state))
            .transpose()
    }

    pub(super) fn replace_authoritative(
        &self,
        versions: &[SkillVersionV1],
        active: &HashMap<String, String>,
    ) -> Result<(), RegistryError> {
        let mut connection = self.connection.lock().map_err(|_| RegistryError::Lock)?;
        let transaction = connection.transaction()?;
        transaction.execute("DELETE FROM active_skills", [])?;
        transaction.execute("DELETE FROM skill_versions", [])?;
        transaction.execute("DELETE FROM skill_evaluations", [])?;
        transaction.execute("DELETE FROM skill_publication_outbox", [])?;
        for version in versions {
            transaction.execute(
                "INSERT INTO skill_versions
                 (version_id, skill_id, parent_version_id, payload_json)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    version.version_id,
                    version.skill_id,
                    version.parent_version_id,
                    serde_json::to_string(version)?
                ],
            )?;
        }
        for (skill_id, version_id) in active {
            transaction.execute(
                "INSERT INTO active_skills (skill_id, active_version_id) VALUES (?1, ?2)",
                params![skill_id, version_id],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }
}
