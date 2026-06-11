//! Local SQLite retention store for persona events.
//!
//! Provides durable client-side storage for persona events, enabling offline
//! boot when the relay is unreachable. Uses `INSERT OR REPLACE` keyed on
//! `(kind, pubkey, d_tag)` for NIP-33 latest-wins semantics.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

/// A retained persona event row.
#[derive(Debug, Clone)]
pub struct RetainedEvent {
    pub kind: u32,
    pub pubkey: String,
    pub d_tag: String,
    pub content: String,
    pub created_at: i64,
    pub raw_event: String,
    pub pending_sync: bool,
}

/// Open (or create) the retention database at the given path.
pub fn open_retention_db(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|e| format!("failed to open retention db: {e}"))?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS persona_events (
            kind INTEGER NOT NULL,
            pubkey TEXT NOT NULL,
            d_tag TEXT NOT NULL,
            content TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            raw_event TEXT NOT NULL,
            pending_sync INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (kind, pubkey, d_tag)
        );",
    )
    .map_err(|e| format!("failed to create retention table: {e}"))?;

    Ok(conn)
}

/// Upsert a persona event into the retention store.
///
/// Only replaces if the new event has a newer or equal `created_at` (NIP-33 semantics).
pub fn retain_event(conn: &Connection, event: &RetainedEvent) -> Result<(), String> {
    conn.execute(
        "INSERT INTO persona_events (kind, pubkey, d_tag, content, created_at, raw_event, pending_sync)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (kind, pubkey, d_tag) DO UPDATE SET
            content = excluded.content,
            created_at = excluded.created_at,
            raw_event = excluded.raw_event,
            pending_sync = excluded.pending_sync
         WHERE excluded.created_at >= persona_events.created_at",
        params![
            event.kind,
            event.pubkey,
            event.d_tag,
            event.content,
            event.created_at,
            event.raw_event,
            event.pending_sync as i32,
        ],
    )
    .map_err(|e| format!("failed to retain event: {e}"))?;

    Ok(())
}

/// Load all retained persona events for a given pubkey.
pub fn get_retained_personas(
    conn: &Connection,
    pubkey: &str,
) -> Result<Vec<RetainedEvent>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT kind, pubkey, d_tag, content, created_at, raw_event, pending_sync
             FROM persona_events
             WHERE pubkey = ?1
             ORDER BY d_tag",
        )
        .map_err(|e| format!("failed to prepare query: {e}"))?;

    let rows = stmt
        .query_map(params![pubkey], |row| {
            Ok(RetainedEvent {
                kind: row.get(0)?,
                pubkey: row.get(1)?,
                d_tag: row.get(2)?,
                content: row.get(3)?,
                created_at: row.get(4)?,
                raw_event: row.get(5)?,
                pending_sync: row.get::<_, i32>(6)? != 0,
            })
        })
        .map_err(|e| format!("failed to query retained events: {e}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("failed to read retained event row: {e}"))
}

/// Get all events marked as pending sync (not yet confirmed on relay).
pub fn get_pending_sync(conn: &Connection) -> Result<Vec<RetainedEvent>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT kind, pubkey, d_tag, content, created_at, raw_event, pending_sync
             FROM persona_events
             WHERE pending_sync = 1",
        )
        .map_err(|e| format!("failed to prepare pending sync query: {e}"))?;

    let rows = stmt
        .query_map([], |row| {
            Ok(RetainedEvent {
                kind: row.get(0)?,
                pubkey: row.get(1)?,
                d_tag: row.get(2)?,
                content: row.get(3)?,
                created_at: row.get(4)?,
                raw_event: row.get(5)?,
                pending_sync: row.get::<_, i32>(6)? != 0,
            })
        })
        .map_err(|e| format!("failed to query pending sync events: {e}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("failed to read pending sync row: {e}"))
}

/// Clear the pending_sync flag for a specific event (after relay confirms).
pub fn mark_synced(conn: &Connection, kind: u32, pubkey: &str, d_tag: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE persona_events SET pending_sync = 0
         WHERE kind = ?1 AND pubkey = ?2 AND d_tag = ?3",
        params![kind, pubkey, d_tag],
    )
    .map_err(|e| format!("failed to mark event synced: {e}"))?;

    Ok(())
}

/// Check if the retention store has any persona events for the given pubkey.
pub fn has_retained_personas(conn: &Connection, pubkey: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM persona_events WHERE pubkey = ?1)",
        params![pubkey],
        |row| row.get(0),
    )
    .map_err(|e| format!("failed to check retained personas: {e}"))
}

/// Look up a single retained event by its coordinate.
pub fn get_retained_event(
    conn: &Connection,
    kind: u32,
    pubkey: &str,
    d_tag: &str,
) -> Result<Option<RetainedEvent>, String> {
    conn.query_row(
        "SELECT kind, pubkey, d_tag, content, created_at, raw_event, pending_sync
         FROM persona_events
         WHERE kind = ?1 AND pubkey = ?2 AND d_tag = ?3",
        params![kind, pubkey, d_tag],
        |row| {
            Ok(RetainedEvent {
                kind: row.get(0)?,
                pubkey: row.get(1)?,
                d_tag: row.get(2)?,
                content: row.get(3)?,
                created_at: row.get(4)?,
                raw_event: row.get(5)?,
                pending_sync: row.get::<_, i32>(6)? != 0,
            })
        },
    )
    .optional()
    .map_err(|e| format!("failed to get retained event: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Connection {
        open_retention_db(Path::new(":memory:")).unwrap()
    }

    fn sample_event() -> RetainedEvent {
        RetainedEvent {
            kind: 30175,
            pubkey: "abc123".to_string(),
            d_tag: "test-persona".to_string(),
            content: r#"{"display_name":"Test"}"#.to_string(),
            created_at: 1000,
            raw_event: r#"{"id":"..."}"#.to_string(),
            pending_sync: true,
        }
    }

    #[test]
    fn retain_and_retrieve() {
        let conn = test_db();
        let event = sample_event();
        retain_event(&conn, &event).unwrap();

        let results = get_retained_personas(&conn, "abc123").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].d_tag, "test-persona");
        assert_eq!(results[0].created_at, 1000);
        assert!(results[0].pending_sync);
    }

    #[test]
    fn upsert_replaces_newer() {
        let conn = test_db();
        let mut event = sample_event();
        retain_event(&conn, &event).unwrap();

        event.content = r#"{"display_name":"Updated"}"#.to_string();
        event.created_at = 2000;
        retain_event(&conn, &event).unwrap();

        let results = get_retained_personas(&conn, "abc123").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].created_at, 2000);
        assert!(results[0].content.contains("Updated"));
    }

    #[test]
    fn upsert_ignores_older() {
        let conn = test_db();
        let mut event = sample_event();
        event.created_at = 2000;
        retain_event(&conn, &event).unwrap();

        event.content = r#"{"display_name":"Old"}"#.to_string();
        event.created_at = 1000;
        retain_event(&conn, &event).unwrap();

        let results = get_retained_personas(&conn, "abc123").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].created_at, 2000);
        assert!(!results[0].content.contains("Old"));
    }

    #[test]
    fn pending_sync_query() {
        let conn = test_db();
        let mut event = sample_event();
        event.pending_sync = true;
        retain_event(&conn, &event).unwrap();

        let mut event2 = sample_event();
        event2.d_tag = "other".to_string();
        event2.pending_sync = false;
        retain_event(&conn, &event2).unwrap();

        let pending = get_pending_sync(&conn).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].d_tag, "test-persona");
    }

    #[test]
    fn mark_synced_clears_flag() {
        let conn = test_db();
        let event = sample_event();
        retain_event(&conn, &event).unwrap();

        mark_synced(&conn, 30175, "abc123", "test-persona").unwrap();

        let pending = get_pending_sync(&conn).unwrap();
        assert!(pending.is_empty());

        let results = get_retained_personas(&conn, "abc123").unwrap();
        assert_eq!(results.len(), 1);
        assert!(!results[0].pending_sync);
    }

    #[test]
    fn has_retained_personas_works() {
        let conn = test_db();
        assert!(!has_retained_personas(&conn, "abc123").unwrap());

        let event = sample_event();
        retain_event(&conn, &event).unwrap();

        assert!(has_retained_personas(&conn, "abc123").unwrap());
        assert!(!has_retained_personas(&conn, "other").unwrap());
    }

    #[test]
    fn get_retained_event_by_coordinate() {
        let conn = test_db();
        let event = sample_event();
        retain_event(&conn, &event).unwrap();

        let found = get_retained_event(&conn, 30175, "abc123", "test-persona").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().d_tag, "test-persona");

        let not_found = get_retained_event(&conn, 30175, "abc123", "nonexistent").unwrap();
        assert!(not_found.is_none());
    }

    #[test]
    fn idempotent_retain_same_timestamp() {
        let conn = test_db();
        let event = sample_event();
        retain_event(&conn, &event).unwrap();
        retain_event(&conn, &event).unwrap();

        let results = get_retained_personas(&conn, "abc123").unwrap();
        assert_eq!(results.len(), 1);
    }
}
