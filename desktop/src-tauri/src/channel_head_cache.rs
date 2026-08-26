//! Persistent native cache for recently visited channel head pages.
//!
//! The cache is a paint accelerator only: the renderer always replaces a
//! hydrated page with an authoritative relay response after subscribing.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager, State};

const SCHEMA_VERSION: i64 = 1;
const CHANNELS_PER_SCOPE_CAP: i64 = 32;
const ROW_BYTES_CAP: usize = 1024 * 1024;

/// Serializes cache mutations on the blocking pool.
#[derive(Default)]
pub(crate) struct ChannelHeadCacheStore {
    write_lock: Arc<Mutex<()>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChannelHeadScope {
    pub(crate) pubkey: String,
    pub(crate) relay_url: String,
}

impl ChannelHeadScope {
    fn key(&self) -> String {
        format!(
            "{}:{}",
            self.pubkey.trim().to_ascii_lowercase(),
            self.relay_url.trim().trim_end_matches('/')
        )
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChannelHeadEntry {
    channel_id: String,
    events: Vec<Value>,
    saved_at: i64,
    last_visited_at: i64,
}

fn db_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("resolve channel-head cache data dir: {error}"))?;
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("create channel-head cache data dir: {error}"))?;
    Ok(dir.join("channel-head-cache.db"))
}

fn create_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE schema_meta(version INTEGER NOT NULL);
         INSERT INTO schema_meta(version) VALUES(1);
         CREATE TABLE channel_head(
           scope TEXT NOT NULL,
           channel_id TEXT NOT NULL,
           events_json TEXT NOT NULL,
           row_count INTEGER NOT NULL,
           saved_at INTEGER NOT NULL,
           last_visited_at INTEGER NOT NULL,
           PRIMARY KEY(scope, channel_id)
         );",
    )
    .map_err(|error| format!("initialize channel-head cache db: {error}"))
}

fn open_db(path: &Path) -> Result<Connection, String> {
    let conn =
        Connection::open(path).map_err(|error| format!("open channel-head cache db: {error}"))?;
    conn.pragma_update(None, "busy_timeout", 5_000)
        .map_err(|error| format!("configure channel-head cache db: {error}"))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|error| format!("configure channel-head cache WAL: {error}"))?;

    let has_schema_meta: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_meta')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("inspect channel-head cache schema: {error}"))?;
    if !has_schema_meta {
        create_schema(&conn)?;
        return Ok(conn);
    }

    let version = conn
        .query_row("SELECT version FROM schema_meta LIMIT 1", [], |row| {
            row.get::<_, i64>(0)
        })
        .optional()
        .map_err(|error| format!("read channel-head cache schema: {error}"))?;
    let has_channel_head: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='channel_head')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("inspect channel-head cache table: {error}"))?;
    if version != Some(SCHEMA_VERSION) || !has_channel_head {
        conn.execute_batch("DROP TABLE IF EXISTS channel_head; DROP TABLE IF EXISTS schema_meta;")
            .map_err(|error| format!("reset channel-head cache schema: {error}"))?;
        create_schema(&conn)?;
    }
    Ok(conn)
}

async fn run_blocking<T, F>(task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|error| format!("channel-head cache db task failed: {error}"))?
}

fn load_from_path(
    path: &Path,
    scope: &ChannelHeadScope,
    limit: u32,
) -> Result<Vec<ChannelHeadEntry>, String> {
    let conn = open_db(path)?;
    let mut statement = conn
        .prepare(
            "SELECT channel_id, events_json, saved_at, last_visited_at
             FROM channel_head WHERE scope=?1
             ORDER BY last_visited_at DESC, saved_at DESC, channel_id ASC LIMIT ?2",
        )
        .map_err(|error| format!("prepare channel-head cache load: {error}"))?;
    let rows = statement
        .query_map(params![scope.key(), i64::from(limit)], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(|error| format!("query channel-head cache: {error}"))?;
    let mut entries = Vec::new();
    for row in rows {
        let (channel_id, events_json, saved_at, last_visited_at) =
            row.map_err(|error| format!("read channel-head cache row: {error}"))?;
        let events = match serde_json::from_str(&events_json) {
            Ok(events) => events,
            Err(error) => {
                eprintln!("skipping corrupt channel-head cache row {channel_id}: {error}");
                continue;
            }
        };
        entries.push(ChannelHeadEntry {
            channel_id,
            events,
            saved_at,
            last_visited_at,
        });
    }
    Ok(entries)
}

fn store_in_transaction(
    transaction: &Transaction<'_>,
    scope: &str,
    channel_id: &str,
    events_json: &str,
    row_count: usize,
    now: i64,
) -> Result<(), String> {
    let last_visited_at: i64 = transaction
        .query_row(
            "SELECT COALESCE(MAX(last_visited_at), ?2 - 1) + 1 FROM channel_head WHERE scope=?1",
            params![scope, now],
            |row| row.get(0),
        )
        .map_err(|error| format!("advance channel-head cache visit clock: {error}"))?;
    transaction
        .execute(
            "INSERT INTO channel_head(scope, channel_id, events_json, row_count, saved_at, last_visited_at)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(scope, channel_id) DO UPDATE SET
               events_json=excluded.events_json,
               row_count=excluded.row_count,
               saved_at=excluded.saved_at,
               last_visited_at=excluded.last_visited_at",
            params![scope, channel_id, events_json, row_count as i64, now, last_visited_at],
        )
        .map_err(|error| format!("store channel-head cache row: {error}"))?;
    transaction
        .execute(
            "DELETE FROM channel_head WHERE rowid IN (
               SELECT rowid FROM channel_head WHERE scope=?1
               ORDER BY last_visited_at DESC, saved_at DESC, channel_id ASC
               LIMIT -1 OFFSET ?2
             )",
            params![scope, CHANNELS_PER_SCOPE_CAP],
        )
        .map_err(|error| format!("prune channel-head cache: {error}"))?;
    Ok(())
}

fn store_at(
    path: &Path,
    scope: &ChannelHeadScope,
    channel_id: &str,
    events: &[Value],
    now: i64,
) -> Result<(), String> {
    let events_json = serde_json::to_string(events)
        .map_err(|error| format!("encode channel-head cache row: {error}"))?;
    let mut conn = open_db(path)?;
    let transaction = conn
        .transaction()
        .map_err(|error| format!("begin channel-head cache store: {error}"))?;
    if events_json.len() > ROW_BYTES_CAP {
        transaction
            .execute(
                "DELETE FROM channel_head WHERE scope=?1 AND channel_id=?2",
                params![scope.key(), channel_id],
            )
            .map_err(|error| format!("drop oversized channel-head cache row: {error}"))?;
    } else {
        store_in_transaction(
            &transaction,
            &scope.key(),
            channel_id,
            &events_json,
            events.len(),
            now,
        )?;
    }
    transaction
        .commit()
        .map_err(|error| format!("commit channel-head cache store: {error}"))
}

/// Loads the most recently visited channel heads for one identity and relay.
#[tauri::command]
pub(crate) async fn channel_head_cache_load(
    scope: ChannelHeadScope,
    limit: u32,
    app: AppHandle,
) -> Result<Vec<ChannelHeadEntry>, String> {
    let path = db_path(&app)?;
    run_blocking(move || load_from_path(&path, &scope, limit)).await
}

/// Stores one raw channel-window response, dropping payloads above one MiB.
#[tauri::command]
pub(crate) async fn channel_head_cache_store(
    scope: ChannelHeadScope,
    channel_id: String,
    events: Vec<Value>,
    app: AppHandle,
    store: State<'_, ChannelHeadCacheStore>,
) -> Result<(), String> {
    let path = db_path(&app)?;
    let write_lock = Arc::clone(&store.write_lock);
    run_blocking(move || {
        let _guard = write_lock.lock().map_err(|error| error.to_string())?;
        store_at(
            &path,
            &scope,
            &channel_id,
            &events,
            chrono::Utc::now().timestamp(),
        )
    })
    .await
}

/// Clears all persisted channel heads for one identity and relay.
#[tauri::command]
pub(crate) async fn channel_head_cache_clear(
    scope: ChannelHeadScope,
    app: AppHandle,
    store: State<'_, ChannelHeadCacheStore>,
) -> Result<(), String> {
    let path = db_path(&app)?;
    let write_lock = Arc::clone(&store.write_lock);
    run_blocking(move || {
        let _guard = write_lock.lock().map_err(|error| error.to_string())?;
        let conn = open_db(&path)?;
        conn.execute("DELETE FROM channel_head WHERE scope=?1", [scope.key()])
            .map_err(|error| format!("clear channel-head cache scope: {error}"))?;
        Ok(())
    })
    .await
}

pub(crate) fn flush(app: &AppHandle) {
    if let Ok(path) = db_path(app) {
        if let Ok(conn) = open_db(&path) {
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope() -> ChannelHeadScope {
        ChannelHeadScope {
            pubkey: "PK".into(),
            relay_url: "wss://relay/".into(),
        }
    }

    #[test]
    fn serialized_entry_matches_typescript_contract() {
        let actual = serde_json::to_value(ChannelHeadEntry {
            channel_id: "general".into(),
            events: vec![serde_json::json!({"id":"event"})],
            saved_at: 42,
            last_visited_at: 43,
        })
        .unwrap();
        let expected = serde_json::json!({
            "channelId":"general",
            "events":[{"id":"event"}],
            "savedAt":42,
            "lastVisitedAt":43
        });
        assert_eq!(actual, expected);

        let decoded: ChannelHeadScope = serde_json::from_value(serde_json::json!({
            "pubkey":"PK", "relayUrl":"wss://relay/"
        }))
        .unwrap();
        assert_eq!(decoded, scope());
        assert_eq!(decoded.key(), "pk:wss://relay");
    }

    #[test]
    fn enforces_lru_and_payload_caps() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("channel-head-cache.db");
        for index in 0..=CHANNELS_PER_SCOPE_CAP {
            store_at(
                &path,
                &scope(),
                &format!("channel-{index:02}"),
                &[serde_json::json!({"index":index})],
                1_000 + index,
            )
            .unwrap();
        }
        let entries = load_from_path(&path, &scope(), 100).unwrap();
        assert_eq!(entries.len(), CHANNELS_PER_SCOPE_CAP as usize);
        assert_eq!(entries.first().unwrap().channel_id, "channel-32");
        assert!(!entries.iter().any(|entry| entry.channel_id == "channel-00"));

        let oversized = vec![Value::String("x".repeat(ROW_BYTES_CAP))];
        store_at(&path, &scope(), "channel-32", &oversized, 2_000).unwrap();
        let count: i64 = open_db(&path)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM channel_head WHERE channel_id='channel-32'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn skips_corrupt_rows_without_blanketing_good_entries() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("channel-head-cache.db");
        store_at(
            &path,
            &scope(),
            "good-channel",
            &[serde_json::json!({"id":"good-event"})],
            1_000,
        )
        .unwrap();
        let conn = open_db(&path).unwrap();
        conn.execute(
            "INSERT INTO channel_head VALUES(?1, 'bad-channel', 'not-json', 1, 1001, 1001)",
            [scope().key()],
        )
        .unwrap();
        drop(conn);

        let entries = load_from_path(&path, &scope(), 12).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].channel_id, "good-channel");
        assert_eq!(
            entries[0].events,
            vec![serde_json::json!({"id":"good-event"})]
        );
    }

    #[test]
    fn schema_mismatch_recreates_cache() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("channel-head-cache.db");
        let conn = open_db(&path).unwrap();
        conn.execute("UPDATE schema_meta SET version=99", [])
            .unwrap();
        conn.execute(
            "INSERT INTO channel_head VALUES('scope','channel','[]',0,1,1)",
            [],
        )
        .unwrap();
        drop(conn);

        let reset = open_db(&path).unwrap();
        let version: i64 = reset
            .query_row("SELECT version FROM schema_meta", [], |row| row.get(0))
            .unwrap();
        let rows: i64 = reset
            .query_row("SELECT COUNT(*) FROM channel_head", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        assert_eq!(rows, 0);
    }
}
