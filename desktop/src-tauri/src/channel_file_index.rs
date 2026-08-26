//! Persistent local index of channel files.
//!
//! Moves the Files-tab read off the network: instead of re-scanning a
//! channel's whole history on every open (see `listChannelFiles` in
//! `desktop/src/shared/api/channelFiles.ts`), files are indexed into SQLite as
//! their events flow through the client, and the tab reads pre-computed rows.
//!
//! This is Phase 1 — the store only. It is wired to nothing yet; Phase 2
//! backfills and hooks ingestion, Phase 3 points the Files tab here. See
//! `docs/channel-file-index-design.md`.
//!
//! Store conventions mirror `archive/store.rs`: open a fresh `Connection` per
//! operation (no live connection in `AppState`), `busy_timeout=5000`, WAL with
//! the same retry-on-busy loop, `CREATE ... IF NOT EXISTS` schema, and a marker
//! migrations table rather than `PRAGMA user_version`. Rows are scoped by
//! `identity_pubkey` so switching accounts never leaks another's file list.
#![allow(dead_code)] // Phases 2-3 wire these in; remove then.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::app_state::AppState;

/// Channel content-message kinds that can carry an `imeta` attachment. Mirrors
/// `KIND_STREAM_MESSAGE`/`_V2` used everywhere else in the timeline path.
const KIND_STREAM_MESSAGE: u16 = 9;
const KIND_STREAM_MESSAGE_V2: u16 = 40002;
/// Relay system message; carries the `message_deleted` tombstone.
const KIND_SYSTEM_MESSAGE: u16 = 40099;

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS channel_file_index (
    identity_pubkey TEXT NOT NULL,
    channel_id      TEXT NOT NULL,
    event_id        TEXT NOT NULL,
    url             TEXT NOT NULL DEFAULT '',
    kind            TEXT NOT NULL,
    uploaded_by     TEXT NOT NULL,
    uploaded_at     INTEGER NOT NULL,
    filename        TEXT,
    sha256          TEXT,
    size            INTEGER,
    mime            TEXT,
    supersedes      TEXT,
    deleted         INTEGER NOT NULL DEFAULT 0,
    indexed_at      INTEGER NOT NULL,
    PRIMARY KEY (identity_pubkey, channel_id, event_id, url)
);

CREATE INDEX IF NOT EXISTS channel_file_index_by_channel
    ON channel_file_index (identity_pubkey, channel_id, deleted, uploaded_at DESC, event_id);

CREATE TABLE IF NOT EXISTS channel_file_tombstones (
    identity_pubkey TEXT NOT NULL,
    channel_id      TEXT NOT NULL,
    event_id        TEXT NOT NULL,
    PRIMARY KEY (identity_pubkey, channel_id, event_id)
);

-- Per-channel high-water mark: the newest event `created_at` already indexed,
-- so an incremental sync only has to fetch events after it.
CREATE TABLE IF NOT EXISTS channel_file_index_cursor (
    identity_pubkey  TEXT NOT NULL,
    channel_id       TEXT NOT NULL,
    last_created_at  INTEGER NOT NULL,
    PRIMARY KEY (identity_pubkey, channel_id)
);

CREATE TABLE IF NOT EXISTS channel_file_index_migrations (
    name       TEXT PRIMARY KEY,
    applied_at INTEGER NOT NULL
);
";

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Set WAL, retrying while another connection holds the lock — mirrors
/// `archive/store.rs`'s `set_wal_mode` (WAL is a global file-level change that
/// can transiently fail with `DatabaseBusy`/`DatabaseLocked`).
fn set_wal_mode(conn: &Connection) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match conn.pragma_update(None, "journal_mode", "WAL") {
            Ok(()) => return Ok(()),
            Err(error) => {
                let busy = matches!(
                    error.sqlite_error_code(),
                    Some(rusqlite::ErrorCode::DatabaseBusy)
                        | Some(rusqlite::ErrorCode::DatabaseLocked)
                );
                if busy && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }
                return Err(format!("could not set WAL mode: {error}"));
            }
        }
    }
}

/// Open (creating if needed) the channel file-index database at `path`.
pub(crate) fn open_db(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create file-index dir: {error}"))?;
    }
    let conn = Connection::open(path)
        .map_err(|error| format!("could not open file-index db: {error}"))?;
    conn.pragma_update(None, "busy_timeout", 5000)
        .map_err(|error| format!("could not set busy_timeout: {error}"))?;
    set_wal_mode(&conn)?;
    conn.execute_batch(SCHEMA)
        .map_err(|error| format!("could not apply file-index schema: {error}"))?;
    apply_schema_migrations(&conn)?;
    Ok(conn)
}

/// No schema migrations yet — Phase 1 ships the table fresh. The marker table
/// (`channel_file_index_migrations`) already exists in `SCHEMA`; future changes
/// add guarded, last-committed migrations here exactly as `archive` does.
fn apply_schema_migrations(_conn: &Connection) -> Result<(), String> {
    Ok(())
}

/// A relay event reduced to what the index needs. Built trivially from
/// `nostr::Event` (see `from_nostr`), and cheap for tests to construct.
#[derive(Clone, Debug)]
pub(crate) struct IndexableEvent {
    pub id: String,
    pub kind: u16,
    pub pubkey: String,
    pub content: String,
    pub created_at: i64,
    pub tags: Vec<Vec<String>>,
}

impl IndexableEvent {
    /// Same reduction `unread_catch_up`'s `EventView` uses, so it is known to
    /// compile against this `nostr` version.
    pub(crate) fn from_nostr(event: &nostr::Event) -> Self {
        Self {
            id: event.id.to_hex(),
            kind: event.kind.as_u16(),
            pubkey: event.pubkey.to_hex(),
            content: event.content.clone(),
            created_at: event.created_at.as_secs() as i64,
            tags: event
                .tags
                .iter()
                .map(|tag| tag.as_slice().to_vec())
                .collect(),
        }
    }
}

/// The fields we pull out of one `imeta` tag. Mirrors `parseImetaTags`
/// (`desktop/src/shared/ui/markdown/parseImeta.ts`): `imeta` tag members after
/// the first are `"<key> <value>"` strings; only entries carrying a `url` count.
struct ImetaFields {
    url: String,
    mime: Option<String>,
    sha256: Option<String>,
    size: Option<i64>,
    filename: Option<String>,
}

fn parse_imeta(tags: &[Vec<String>]) -> Vec<ImetaFields> {
    let mut out = Vec::new();
    for tag in tags {
        if tag.first().map(String::as_str) != Some("imeta") {
            continue;
        }
        let mut url: Option<String> = None;
        let mut mime: Option<String> = None;
        let mut sha256: Option<String> = None;
        let mut size: Option<i64> = None;
        let mut filename: Option<String> = None;
        for part in tag.iter().skip(1) {
            let Some(space) = part.find(' ') else {
                continue;
            };
            let key = &part[..space];
            let value = &part[space + 1..];
            match key {
                "url" => url = Some(value.to_string()),
                "m" => mime = Some(value.to_string()),
                "x" => sha256 = Some(value.to_string()),
                "size" => size = value.parse::<i64>().ok(),
                "filename" => filename = Some(value.to_string()),
                _ => {}
            }
        }
        // The frontend keys entries by `url` and drops any without one.
        if let Some(url) = url {
            out.push(ImetaFields {
                url,
                mime,
                sha256,
                size,
                filename,
            });
        }
    }
    out
}

/// Read the `["e", "<id>", "<relay>", "supersedes"]` marker, if present.
/// Mirrors `supersedesTarget` in `channelFiles.ts`.
fn parse_supersedes(tags: &[Vec<String>]) -> Option<String> {
    for tag in tags {
        if tag.first().map(String::as_str) != Some("e") {
            continue;
        }
        if tag.get(3).map(String::as_str) == Some("supersedes") {
            if let Some(id) = tag.get(1) {
                if !id.is_empty() {
                    return Some(id.clone());
                }
            }
        }
    }
    None
}

/// Read a `message_deleted` tombstone's target event id from a kind-40099 body.
fn parse_tombstone(content: &str) -> Option<String> {
    let payload: serde_json::Value = serde_json::from_str(content).ok()?;
    if payload.get("type")?.as_str()? != "message_deleted" {
        return None;
    }
    payload
        .get("target_event_id")?
        .as_str()
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

/// Index a batch of events for `(identity_pubkey, channel_id)`. Idempotent:
/// re-seeing an event is a no-op, so backfill, live and catch-up can overlap.
/// Returns the number of file rows written.
pub(crate) fn index_events(
    conn: &Connection,
    identity_pubkey: &str,
    channel_id: &str,
    events: &[IndexableEvent],
) -> Result<usize, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("could not begin file-index tx: {error}"))?;
    let indexed_at = now_secs();
    let mut written = 0usize;

    for event in events {
        match event.kind {
            KIND_STREAM_MESSAGE | KIND_STREAM_MESSAGE_V2 => {
                let supersedes = parse_supersedes(&event.tags);
                for entry in parse_imeta(&event.tags) {
                    // `deleted` is set from the tombstone table so a tombstone
                    // seen before its file (out-of-order paging) still wins.
                    tx.execute(
                        "INSERT INTO channel_file_index (
                            identity_pubkey, channel_id, event_id, url, kind,
                            uploaded_by, uploaded_at, filename, sha256, size, mime,
                            supersedes, deleted, indexed_at
                         ) VALUES (
                            ?1, ?2, ?3, ?4, 'file',
                            ?5, ?6, ?7, ?8, ?9, ?10,
                            ?11,
                            (SELECT CASE WHEN EXISTS(
                                SELECT 1 FROM channel_file_tombstones t
                                WHERE t.identity_pubkey = ?1 AND t.channel_id = ?2
                                  AND t.event_id = ?3
                             ) THEN 1 ELSE 0 END),
                            ?12
                         )
                         ON CONFLICT(identity_pubkey, channel_id, event_id, url)
                         DO UPDATE SET
                            uploaded_by = excluded.uploaded_by,
                            uploaded_at = excluded.uploaded_at,
                            filename    = excluded.filename,
                            sha256      = excluded.sha256,
                            size        = excluded.size,
                            mime        = excluded.mime,
                            supersedes  = excluded.supersedes,
                            indexed_at  = excluded.indexed_at",
                        params![
                            identity_pubkey,
                            channel_id,
                            event.id,
                            entry.url,
                            event.pubkey,
                            event.created_at,
                            entry.filename,
                            entry.sha256,
                            entry.size,
                            entry.mime,
                            supersedes,
                            indexed_at,
                        ],
                    )
                    .map_err(|error| format!("could not upsert file row: {error}"))?;
                    written += 1;
                }
            }
            KIND_SYSTEM_MESSAGE => {
                if let Some(target) = parse_tombstone(&event.content) {
                    tx.execute(
                        "INSERT OR IGNORE INTO channel_file_tombstones
                            (identity_pubkey, channel_id, event_id)
                         VALUES (?1, ?2, ?3)",
                        params![identity_pubkey, channel_id, target],
                    )
                    .map_err(|error| format!("could not record tombstone: {error}"))?;
                    tx.execute(
                        "UPDATE channel_file_index SET deleted = 1
                         WHERE identity_pubkey = ?1 AND channel_id = ?2 AND event_id = ?3",
                        params![identity_pubkey, channel_id, target],
                    )
                    .map_err(|error| format!("could not apply tombstone: {error}"))?;
                }
            }
            _ => {}
        }
    }

    tx.commit()
        .map_err(|error| format!("could not commit file-index tx: {error}"))?;
    Ok(written)
}

/// One file row as the Files tab consumes it. Field names match the TS
/// `ChannelFileEntry` (`camelCase`) so it deserializes with no mapping layer.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChannelFileRow {
    pub kind: String,
    pub event_id: String,
    pub uploaded_by: String,
    pub uploaded_at: i64,
    pub filename: Option<String>,
    pub sha256: Option<String>,
    pub size: Option<i64>,
    pub mime: Option<String>,
    pub url: Option<String>,
    pub supersedes: Option<String>,
    pub superseded_by: Option<String>,
}

/// Every surviving file in a channel, newest upload first, with version-chain
/// links resolved — the same output shape and two-pass linkage
/// `channelFiles.ts` produces, just read from the index.
pub(crate) fn query_channel_files(
    conn: &Connection,
    identity_pubkey: &str,
    channel_id: &str,
) -> Result<Vec<ChannelFileRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT kind, event_id, uploaded_by, uploaded_at, filename, sha256,
                    size, mime, url, supersedes
             FROM channel_file_index
             WHERE identity_pubkey = ?1 AND channel_id = ?2 AND deleted = 0
             ORDER BY uploaded_at DESC, event_id",
        )
        .map_err(|error| format!("could not prepare file query: {error}"))?;

    let mut rows: Vec<ChannelFileRow> = stmt
        .query_map(params![identity_pubkey, channel_id], |row| {
            let url: Option<String> = row.get(8)?;
            Ok(ChannelFileRow {
                kind: row.get(0)?,
                event_id: row.get(1)?,
                uploaded_by: row.get(2)?,
                uploaded_at: row.get(3)?,
                filename: row.get(4)?,
                sha256: row.get(5)?,
                size: row.get(6)?,
                mime: row.get(7)?,
                // An empty url is stored as '' (PK can't be NULL); present it as
                // null to match the frontend's `url: string | null`.
                url: url.filter(|value| !value.is_empty()),
                supersedes: row.get(9)?,
                superseded_by: None, // filled below
            })
        })
        .map_err(|error| format!("could not run file query: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("could not read file rows: {error}"))?;

    // Second pass, mirroring `channelFiles.ts`: back-fill `supersededBy` from
    // the `supersedes` graph, then drop links whose other end isn't present
    // (deleted or never-indexed), so no badge points at a file nobody can see.
    let present: std::collections::HashSet<String> =
        rows.iter().map(|row| row.event_id.clone()).collect();

    // newer event_id -> older (superseded) event_id, from surviving rows only.
    let mut superseded_by: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for row in &rows {
        if let Some(older) = &row.supersedes {
            if present.contains(older) {
                superseded_by.insert(older.clone(), row.event_id.clone());
            }
        }
    }

    for row in &mut rows {
        if let Some(older) = &row.supersedes {
            if !present.contains(older) {
                row.supersedes = None;
            }
        }
        if let Some(newer) = superseded_by.get(&row.event_id) {
            row.superseded_by = Some(newer.clone());
        }
    }

    Ok(rows)
}

// ── Watermark cursor ─────────────────────────────────────────────────────────

/// The newest event `created_at` already indexed for a channel, if any.
pub(crate) fn get_channel_cursor(
    conn: &Connection,
    identity_pubkey: &str,
    channel_id: &str,
) -> Result<Option<i64>, String> {
    conn.query_row(
        "SELECT last_created_at FROM channel_file_index_cursor
         WHERE identity_pubkey = ?1 AND channel_id = ?2",
        params![identity_pubkey, channel_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|error| format!("could not read file-index cursor: {error}"))
}

/// Advance a channel's watermark (never moves it backward).
pub(crate) fn set_channel_cursor(
    conn: &Connection,
    identity_pubkey: &str,
    channel_id: &str,
    last_created_at: i64,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO channel_file_index_cursor
            (identity_pubkey, channel_id, last_created_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(identity_pubkey, channel_id)
         DO UPDATE SET last_created_at = MAX(last_created_at, excluded.last_created_at)",
        params![identity_pubkey, channel_id, last_created_at],
    )
    .map_err(|error| format!("could not write file-index cursor: {error}"))?;
    Ok(())
}

// ── Sync (backfill + incremental) ────────────────────────────────────────────

/// Event kinds the index cares about: content messages (which may carry an
/// `imeta` attachment) and the deletion tombstone. No `top_level` — this is the
/// non-scoped channel query (same shape as `channel_reconnect_repair`) that
/// returns thread replies too.
const SCAN_KINDS: [u32; 3] = [
    KIND_STREAM_MESSAGE as u32,
    KIND_STREAM_MESSAGE_V2 as u32,
    KIND_SYSTEM_MESSAGE as u32,
];

/// One relay `/query` page. Capped like `get_channel_messages_before`.
const SCAN_PAGE_SIZE: u32 = 500;

/// Page ceiling — the same malformed-cursor guard `listChannelFiles` uses.
const MAX_SCAN_PAGES: usize = 200;

fn db_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("could not resolve file-index data dir: {error}"))?;
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("could not create file-index data dir: {error}"))?;
    Ok(dir.join("channel-file-index.db"))
}

fn identity_pubkey(state: &AppState) -> Result<String, String> {
    let keys = state.keys.lock().map_err(|error| error.to_string())?;
    Ok(keys.public_key().to_hex())
}

/// Page the channel's history backward (newest first) via the non-`top_level`
/// filter, converting to `IndexableEvent`. Stops when history is exhausted or
/// (for an incremental sync) once a page's oldest event is at/older than
/// `watermark`. Returns the events plus the new high-water mark to persist.
///
/// Deliberately does all network I/O first and touches no `Connection`, so no
/// non-`Send` SQLite handle is ever held across an `await`.
async fn fetch_channel_events_since(
    state: &AppState,
    channel_id: &str,
    watermark: i64,
) -> Result<(Vec<IndexableEvent>, i64), String> {
    let mut collected: Vec<IndexableEvent> = Vec::new();
    let mut until: Option<i64> = None;
    let mut before_id: Option<String> = None;
    let mut new_watermark = watermark;

    for _ in 0..MAX_SCAN_PAGES {
        let mut filter = serde_json::Map::new();
        filter.insert("#h".to_string(), serde_json::json!([channel_id]));
        filter.insert("kinds".to_string(), serde_json::json!(SCAN_KINDS));
        filter.insert("limit".to_string(), serde_json::json!(SCAN_PAGE_SIZE));
        if let (Some(u), Some(b)) = (until, &before_id) {
            // The relay's keyset requires both `until` and `before_id` together.
            filter.insert("until".to_string(), serde_json::json!(u));
            filter.insert("before_id".to_string(), serde_json::json!(b));
        }

        let events =
            crate::relay::query_relay(state, &[serde_json::Value::Object(filter)]).await?;
        if events.is_empty() {
            break;
        }

        // Relay order is `created_at DESC, id ASC`; sort to be certain so the
        // last element is unambiguously the oldest — the next keyset cursor.
        let mut page: Vec<IndexableEvent> =
            events.iter().map(IndexableEvent::from_nostr).collect();
        page.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        let page_len = page.len();
        let oldest = page.last().expect("page is non-empty");
        let oldest_created_at = oldest.created_at;
        until = Some(oldest.created_at);
        before_id = Some(oldest.id.clone());

        for event in &page {
            if event.created_at > new_watermark {
                new_watermark = event.created_at;
            }
        }
        collected.extend(page);

        if page_len < SCAN_PAGE_SIZE as usize {
            break; // history exhausted
        }
        if oldest_created_at <= watermark {
            break; // reached the incremental floor
        }
    }

    Ok((collected, new_watermark))
}

/// Bring the file index for one channel up to date, then return how many file
/// rows were written. First call for a channel backfills its whole history
/// (watermark 0); later calls fetch only what arrived since.
#[tauri::command]
pub(crate) async fn sync_channel_file_index(
    channel_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<usize, String> {
    let identity = identity_pubkey(&state)?;
    let path = db_path(&app)?;

    let watermark = {
        let path = path.clone();
        let identity = identity.clone();
        let channel_id = channel_id.clone();
        tokio::task::spawn_blocking(move || -> Result<i64, String> {
            let conn = open_db(&path)?;
            Ok(get_channel_cursor(&conn, &identity, &channel_id)?.unwrap_or(0))
        })
        .await
        .map_err(|error| format!("spawn_blocking failed: {error}"))??
    };

    let (events, new_watermark) =
        fetch_channel_events_since(&state, &channel_id, watermark).await?;

    tokio::task::spawn_blocking(move || -> Result<usize, String> {
        let conn = open_db(&path)?;
        let written = index_events(&conn, &identity, &channel_id, &events)?;
        set_channel_cursor(&conn, &identity, &channel_id, new_watermark)?;
        Ok(written)
    })
    .await
    .map_err(|error| format!("spawn_blocking failed: {error}"))?
}

/// Read the indexed files for a channel (newest first, version links resolved).
/// Does no network I/O — the sync command keeps the index current.
#[tauri::command]
pub(crate) async fn list_channel_file_index(
    channel_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Vec<ChannelFileRow>, String> {
    let identity = identity_pubkey(&state)?;
    let path = db_path(&app)?;
    tokio::task::spawn_blocking(move || -> Result<Vec<ChannelFileRow>, String> {
        let conn = open_db(&path)?;
        query_channel_files(&conn, &identity, &channel_id)
    })
    .await
    .map_err(|error| format!("spawn_blocking failed: {error}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "idpub";
    const CH: &str = "chan-1";

    fn mem_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory");
        conn.execute_batch(SCHEMA).expect("schema");
        conn
    }

    fn file_event(id: &str, url: &str, created_at: i64) -> IndexableEvent {
        IndexableEvent {
            id: id.to_string(),
            kind: KIND_STREAM_MESSAGE,
            pubkey: "author".to_string(),
            content: String::new(),
            created_at,
            tags: vec![vec![
                "imeta".to_string(),
                format!("url {url}"),
                "m application/pdf".to_string(),
                "x deadbeef".to_string(),
                "size 2048".to_string(),
                "filename notes.pdf".to_string(),
            ]],
        }
    }

    fn tombstone_event(id: &str, target: &str) -> IndexableEvent {
        IndexableEvent {
            id: id.to_string(),
            kind: KIND_SYSTEM_MESSAGE,
            pubkey: "relay".to_string(),
            content: format!(
                "{{\"type\":\"message_deleted\",\"target_event_id\":\"{target}\"}}"
            ),
            created_at: 0,
            tags: vec![],
        }
    }

    #[test]
    fn indexes_a_file_with_its_imeta_fields() {
        let conn = mem_db();
        index_events(&conn, ID, CH, &[file_event("e1", "https://x/a.pdf", 100)])
            .expect("index");
        let files = query_channel_files(&conn, ID, CH).expect("query");
        assert_eq!(files.len(), 1);
        let f = &files[0];
        assert_eq!(f.kind, "file");
        assert_eq!(f.event_id, "e1");
        assert_eq!(f.url.as_deref(), Some("https://x/a.pdf"));
        assert_eq!(f.mime.as_deref(), Some("application/pdf"));
        assert_eq!(f.sha256.as_deref(), Some("deadbeef"));
        assert_eq!(f.size, Some(2048));
        assert_eq!(f.filename.as_deref(), Some("notes.pdf"));
    }

    #[test]
    fn newest_upload_comes_first() {
        let conn = mem_db();
        index_events(
            &conn,
            ID,
            CH,
            &[
                file_event("old", "https://x/old.pdf", 100),
                file_event("new", "https://x/new.pdf", 200),
            ],
        )
        .expect("index");
        let files = query_channel_files(&conn, ID, CH).expect("query");
        assert_eq!(
            files.iter().map(|f| f.event_id.as_str()).collect::<Vec<_>>(),
            vec!["new", "old"],
        );
    }

    #[test]
    fn reindexing_the_same_event_is_idempotent() {
        let conn = mem_db();
        let event = file_event("e1", "https://x/a.pdf", 100);
        index_events(&conn, ID, CH, std::slice::from_ref(&event)).expect("first");
        index_events(&conn, ID, CH, &[event]).expect("second");
        assert_eq!(query_channel_files(&conn, ID, CH).expect("query").len(), 1);
    }

    #[test]
    fn a_message_with_two_attachments_indexes_both() {
        let conn = mem_db();
        let mut event = file_event("e1", "https://x/a.pdf", 100);
        event.tags.push(vec![
            "imeta".to_string(),
            "url https://x/b.png".to_string(),
            "m image/png".to_string(),
        ]);
        index_events(&conn, ID, CH, &[event]).expect("index");
        let files = query_channel_files(&conn, ID, CH).expect("query");
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn a_tombstone_removes_the_file() {
        let conn = mem_db();
        index_events(&conn, ID, CH, &[file_event("e1", "https://x/a.pdf", 100)])
            .expect("index");
        index_events(&conn, ID, CH, &[tombstone_event("t1", "e1")]).expect("delete");
        assert!(query_channel_files(&conn, ID, CH).expect("query").is_empty());
    }

    #[test]
    fn a_tombstone_seen_before_its_file_still_wins() {
        let conn = mem_db();
        // Out-of-order: delete arrives first, then the file it targets.
        index_events(&conn, ID, CH, &[tombstone_event("t1", "e1")]).expect("delete");
        index_events(&conn, ID, CH, &[file_event("e1", "https://x/a.pdf", 100)])
            .expect("index");
        assert!(query_channel_files(&conn, ID, CH).expect("query").is_empty());
    }

    #[test]
    fn supersedes_resolves_both_directions() {
        let conn = mem_db();
        let mut newer = file_event("v2", "https://x/v2.pdf", 200);
        newer.tags.push(vec![
            "e".to_string(),
            "v1".to_string(),
            String::new(),
            "supersedes".to_string(),
        ]);
        index_events(&conn, ID, CH, &[file_event("v1", "https://x/v1.pdf", 100), newer])
            .expect("index");
        let files = query_channel_files(&conn, ID, CH).expect("query");
        let v1 = files.iter().find(|f| f.event_id == "v1").unwrap();
        let v2 = files.iter().find(|f| f.event_id == "v2").unwrap();
        assert_eq!(v2.supersedes.as_deref(), Some("v1"));
        assert_eq!(v1.superseded_by.as_deref(), Some("v2"));
    }

    #[test]
    fn a_supersedes_link_to_a_deleted_file_is_dropped() {
        let conn = mem_db();
        let mut newer = file_event("v2", "https://x/v2.pdf", 200);
        newer.tags.push(vec![
            "e".to_string(),
            "v1".to_string(),
            String::new(),
            "supersedes".to_string(),
        ]);
        index_events(&conn, ID, CH, &[file_event("v1", "https://x/v1.pdf", 100), newer])
            .expect("index");
        index_events(&conn, ID, CH, &[tombstone_event("t1", "v1")]).expect("delete v1");
        let files = query_channel_files(&conn, ID, CH).expect("query");
        assert_eq!(files.len(), 1);
        // v1 is gone, so v2 must not still claim to supersede it.
        assert_eq!(files[0].event_id, "v2");
        assert_eq!(files[0].supersedes, None);
    }

    #[test]
    fn identity_and_channel_scope_are_isolated() {
        let conn = mem_db();
        index_events(&conn, ID, CH, &[file_event("e1", "https://x/a.pdf", 100)])
            .expect("index");
        assert!(query_channel_files(&conn, "other-id", CH).expect("q").is_empty());
        assert!(query_channel_files(&conn, ID, "other-ch").expect("q").is_empty());
    }

    #[test]
    fn cursor_starts_absent_then_advances() {
        let conn = mem_db();
        assert_eq!(get_channel_cursor(&conn, ID, CH).expect("get"), None);
        set_channel_cursor(&conn, ID, CH, 150).expect("set");
        assert_eq!(get_channel_cursor(&conn, ID, CH).expect("get"), Some(150));
    }

    #[test]
    fn cursor_never_moves_backward() {
        let conn = mem_db();
        set_channel_cursor(&conn, ID, CH, 200).expect("set");
        set_channel_cursor(&conn, ID, CH, 100).expect("set lower");
        assert_eq!(get_channel_cursor(&conn, ID, CH).expect("get"), Some(200));
    }

    #[test]
    fn cursor_is_scoped_per_identity_and_channel() {
        let conn = mem_db();
        set_channel_cursor(&conn, ID, CH, 150).expect("set");
        assert_eq!(get_channel_cursor(&conn, "other-id", CH).expect("get"), None);
        assert_eq!(get_channel_cursor(&conn, ID, "other-ch").expect("get"), None);
    }
}
