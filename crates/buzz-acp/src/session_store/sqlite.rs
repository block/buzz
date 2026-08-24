//! SQLite adapter for [`SessionStore`](super::SessionStore).
//!
//! Schema is embedded and applied at open. This is **not** a relay Postgres
//! migration — do not add anything under the repo `migrations/` directory.

use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use rusqlite::{params, Connection};
use uuid::Uuid;

use super::{
    unix_now_secs, ContextKey, SessionBinding, SessionStore, SessionStoreError, StoreScope,
};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS session_bindings (
  scope_agent TEXT NOT NULL,
  scope_relay TEXT NOT NULL,
  scope_adapter TEXT NOT NULL,
  context_key TEXT NOT NULL,
  session_id TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  last_used_at INTEGER NOT NULL,
  PRIMARY KEY (scope_agent, scope_relay, scope_adapter, context_key)
);
CREATE TABLE IF NOT EXISTS processed_events (
  scope_agent TEXT NOT NULL,
  scope_relay TEXT NOT NULL,
  scope_adapter TEXT NOT NULL,
  event_id TEXT NOT NULL,
  channel_id TEXT NOT NULL,
  processed_at INTEGER NOT NULL,
  PRIMARY KEY (scope_agent, scope_relay, scope_adapter, event_id)
);
CREATE TABLE IF NOT EXISTS session_store_migrations (
  name TEXT PRIMARY KEY,
  applied_at INTEGER NOT NULL
);
"#;

const PROCESSED_EVENT_TTL_SECS: i64 = 30 * 24 * 60 * 60;

/// Production SQLite-backed [`SessionStore`].
pub struct SqliteSessionStore {
    conn: Arc<Mutex<Connection>>,
    scope: StoreScope,
}

impl SqliteSessionStore {
    /// Open (or create) the store at `path` and apply the embedded schema.
    pub fn open(path: &Path, scope: StoreScope) -> Result<Self, SessionStoreError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    SessionStoreError::Io(format!("failed to create session store dir: {e}"))
                })?;
                // Ported from #6088: owner-only store directory. Bindings are not
                // secrets, but they name channels, agent pubkeys and workspace
                // paths, and the default umask commonly leaves them world-readable.
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
                        .map_err(|e| {
                            SessionStoreError::Io(format!(
                                "failed to secure session store dir: {e}"
                            ))
                        })?;
                }
            }
        }

        // Create the database at 0600 *before* SQLite opens it, so its bytes are
        // never briefly world-readable. SQLite derives -wal/-shm permissions from
        // the main database file, so this covers the sidecars too.
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .mode(0o600)
                .open(path)
                .map_err(|e| {
                    SessionStoreError::Io(format!("failed to create session store: {e}"))
                })?;
            // Repair a store created before this hardening landed.
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(
                |e| SessionStoreError::Io(format!("failed to secure session store: {e}")),
            )?;
        }

        let conn = Connection::open(path)
            .map_err(|e| SessionStoreError::Io(format!("failed to open session store: {e}")))?;
        conn.pragma_update(None, "busy_timeout", 5000)
            .map_err(|e| SessionStoreError::Io(format!("failed to set busy_timeout: {e}")))?;
        set_wal_mode(&conn)?;
        conn.execute_batch(SCHEMA)
            .map_err(|e| SessionStoreError::Io(format!("failed to initialize schema: {e}")))?;
        apply_v1_marker(&conn)?;
        prune_old_processed_events(&conn, &scope)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            scope,
        })
    }

    async fn with_conn<T, F>(&self, f: F) -> Result<T, SessionStoreError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection, &StoreScope) -> Result<T, SessionStoreError> + Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        let scope = self.scope.clone();
        tokio::task::spawn_blocking(move || {
            let guard = conn
                .lock()
                .map_err(|e| SessionStoreError::Io(format!("session store poisoned: {e}")))?;
            f(&guard, &scope)
        })
        .await
        .map_err(|e| SessionStoreError::Io(format!("session store worker join: {e}")))?
    }
}

fn set_wal_mode(conn: &Connection) -> Result<(), SessionStoreError> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match conn.pragma_update(None, "journal_mode", "WAL") {
            Ok(()) => return Ok(()),
            Err(error) if sqlite_is_busy(&error) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(error) => {
                return Err(SessionStoreError::Io(format!(
                    "failed to set WAL mode: {error}"
                )));
            }
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

fn apply_v1_marker(conn: &Connection) -> Result<(), SessionStoreError> {
    conn.execute(
        "INSERT OR IGNORE INTO session_store_migrations (name, applied_at) VALUES (?1, ?2)",
        params!["v1", unix_now_secs()],
    )
    .map_err(|e| SessionStoreError::Io(format!("failed to record schema marker: {e}")))?;
    Ok(())
}

fn prune_old_processed_events(
    conn: &Connection,
    scope: &StoreScope,
) -> Result<(), SessionStoreError> {
    let cutoff = unix_now_secs() - PROCESSED_EVENT_TTL_SECS;
    let pruned = conn
        .execute(
            "DELETE FROM processed_events
             WHERE scope_agent = ?1 AND scope_relay = ?2 AND scope_adapter = ?3
               AND processed_at < ?4",
            params![scope.agent_pubkey, scope.relay_url, scope.adapter, cutoff],
        )
        .map_err(|e| SessionStoreError::Io(format!("failed to prune processed events: {e}")))?;
    if pruned > 0 {
        tracing::debug!(pruned, "pruned expired processed_events rows");
    }
    Ok(())
}

fn load_binding_row(
    conn: &Connection,
    scope: &StoreScope,
    key: &ContextKey,
) -> Result<Option<SessionBinding>, SessionStoreError> {
    let wire = key.as_wire();
    let mut stmt = conn
        .prepare(
            "SELECT session_id, created_at, last_used_at FROM session_bindings
             WHERE scope_agent = ?1 AND scope_relay = ?2 AND scope_adapter = ?3
               AND context_key = ?4",
        )
        .map_err(|e| SessionStoreError::Io(e.to_string()))?;
    let mut rows = stmt
        .query(params![
            scope.agent_pubkey,
            scope.relay_url,
            scope.adapter,
            wire
        ])
        .map_err(|e| SessionStoreError::Io(e.to_string()))?;
    let Some(row) = rows
        .next()
        .map_err(|e| SessionStoreError::Corrupt(e.to_string()))?
    else {
        return Ok(None);
    };
    let session_id: String = row
        .get(0)
        .map_err(|e| SessionStoreError::Corrupt(e.to_string()))?;
    let created_at: i64 = row
        .get(1)
        .map_err(|e| SessionStoreError::Corrupt(e.to_string()))?;
    let last_used_at: i64 = row
        .get(2)
        .map_err(|e| SessionStoreError::Corrupt(e.to_string()))?;
    if session_id.trim().is_empty() {
        tracing::warn!(
            context_key = %wire,
            "discarding malformed session binding with empty session_id"
        );
        conn.execute(
            "DELETE FROM session_bindings
             WHERE scope_agent = ?1 AND scope_relay = ?2 AND scope_adapter = ?3
               AND context_key = ?4",
            params![scope.agent_pubkey, scope.relay_url, scope.adapter, wire],
        )
        .map_err(|e| SessionStoreError::Io(e.to_string()))?;
        return Ok(None);
    }
    Ok(Some(SessionBinding {
        session_id,
        created_at,
        last_used_at,
    }))
}

#[async_trait]
impl SessionStore for SqliteSessionStore {
    async fn load_binding(
        &self,
        key: &ContextKey,
    ) -> Result<Option<SessionBinding>, SessionStoreError> {
        let key = key.clone();
        self.with_conn(move |conn, scope| load_binding_row(conn, scope, &key))
            .await
    }

    async fn save_binding(
        &self,
        key: &ContextKey,
        session_id: &str,
    ) -> Result<(), SessionStoreError> {
        let key = key.as_wire();
        let session_id = session_id.to_string();
        self.with_conn(move |conn, scope| {
            let now = unix_now_secs();
            conn.execute(
                "INSERT INTO session_bindings (
                    scope_agent, scope_relay, scope_adapter, context_key,
                    session_id, created_at, last_used_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
                 ON CONFLICT(scope_agent, scope_relay, scope_adapter, context_key)
                 DO UPDATE SET session_id = excluded.session_id,
                               last_used_at = excluded.last_used_at",
                params![
                    scope.agent_pubkey,
                    scope.relay_url,
                    scope.adapter,
                    key,
                    session_id,
                    now
                ],
            )
            .map_err(|e| SessionStoreError::Io(e.to_string()))?;
            Ok(())
        })
        .await
    }

    async fn touch_binding(&self, key: &ContextKey) -> Result<(), SessionStoreError> {
        let key = key.as_wire();
        self.with_conn(move |conn, scope| {
            conn.execute(
                "UPDATE session_bindings SET last_used_at = ?5
                 WHERE scope_agent = ?1 AND scope_relay = ?2 AND scope_adapter = ?3
                   AND context_key = ?4",
                params![
                    scope.agent_pubkey,
                    scope.relay_url,
                    scope.adapter,
                    key,
                    unix_now_secs()
                ],
            )
            .map_err(|e| SessionStoreError::Io(e.to_string()))?;
            Ok(())
        })
        .await
    }

    async fn remove_binding(&self, key: &ContextKey) -> Result<(), SessionStoreError> {
        let key = key.as_wire();
        self.with_conn(move |conn, scope| {
            conn.execute(
                "DELETE FROM session_bindings
                 WHERE scope_agent = ?1 AND scope_relay = ?2 AND scope_adapter = ?3
                   AND context_key = ?4",
                params![scope.agent_pubkey, scope.relay_url, scope.adapter, key],
            )
            .map_err(|e| SessionStoreError::Io(e.to_string()))?;
            Ok(())
        })
        .await
    }

    async fn remove_bindings_for_channel(&self, channel_id: Uuid) -> Result<(), SessionStoreError> {
        self.with_conn(move |conn, scope| {
            conn.execute(
                "DELETE FROM session_bindings
                 WHERE scope_agent = ?1 AND scope_relay = ?2 AND scope_adapter = ?3
                   AND (context_key = ?4 OR context_key LIKE '%:' || ?4)",
                params![
                    scope.agent_pubkey,
                    scope.relay_url,
                    scope.adapter,
                    channel_id.to_string()
                ],
            )
            .map_err(|e| SessionStoreError::Io(e.to_string()))?;
            Ok(())
        })
        .await
    }

    async fn is_event_processed(&self, event_id: &str) -> Result<bool, SessionStoreError> {
        let event_id = event_id.to_string();
        self.with_conn(move |conn, scope| {
            let mut stmt = conn
                .prepare(
                    "SELECT 1 FROM processed_events
                     WHERE scope_agent = ?1 AND scope_relay = ?2 AND scope_adapter = ?3
                       AND event_id = ?4",
                )
                .map_err(|e| SessionStoreError::Io(e.to_string()))?;
            let exists = stmt
                .exists(params![
                    scope.agent_pubkey,
                    scope.relay_url,
                    scope.adapter,
                    event_id
                ])
                .map_err(|e| SessionStoreError::Io(e.to_string()))?;
            Ok(exists)
        })
        .await
    }

    async fn mark_events_processed(
        &self,
        channel_id: Uuid,
        event_ids: &[String],
    ) -> Result<(), SessionStoreError> {
        let channel_id = channel_id.to_string();
        let event_ids = event_ids.to_vec();
        self.with_conn(move |conn, scope| {
            let now = unix_now_secs();
            for event_id in event_ids {
                conn.execute(
                    "INSERT INTO processed_events (
                        scope_agent, scope_relay, scope_adapter,
                        event_id, channel_id, processed_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(scope_agent, scope_relay, scope_adapter, event_id)
                     DO UPDATE SET channel_id = excluded.channel_id,
                                   processed_at = excluded.processed_at",
                    params![
                        scope.agent_pubkey,
                        scope.relay_url,
                        scope.adapter,
                        event_id,
                        channel_id,
                        now
                    ],
                )
                .map_err(|e| SessionStoreError::Io(e.to_string()))?;
            }
            Ok(())
        })
        .await
    }

    async fn processed_event_ids_for_channel(
        &self,
        channel_id: Uuid,
    ) -> Result<Vec<String>, SessionStoreError> {
        let channel_id = channel_id.to_string();
        self.with_conn(move |conn, scope| {
            let mut stmt = conn
                .prepare(
                    "SELECT event_id FROM processed_events
                     WHERE scope_agent = ?1 AND scope_relay = ?2 AND scope_adapter = ?3
                       AND channel_id = ?4",
                )
                .map_err(|e| SessionStoreError::Io(e.to_string()))?;
            let rows = stmt
                .query_map(
                    params![
                        scope.agent_pubkey,
                        scope.relay_url,
                        scope.adapter,
                        channel_id
                    ],
                    |row| row.get(0),
                )
                .map_err(|e| SessionStoreError::Io(e.to_string()))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row.map_err(|e| SessionStoreError::Corrupt(e.to_string()))?);
            }
            Ok(ids)
        })
        .await
    }
}

/// Test helper: raw path used by a store after `open`.
#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn db_path_for_tests(dir: &Path) -> PathBuf {
    dir.join("sessions.db")
}
