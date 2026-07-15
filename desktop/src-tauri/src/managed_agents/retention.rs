//! Local SQLite retention store for persona events.
//!
//! Provides durable client-side storage for persona events, enabling offline
//! boot when the relay is unreachable. Upserts via `ON CONFLICT DO UPDATE`
//! keyed on `(kind, pubkey, d_tag)`, replacing only on a newer-or-equal
//! `created_at` for NIP-33 latest-wins semantics.

use std::path::Path;
use std::time::{Duration, Instant};

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
///
/// Sets WAL journaling and a `busy_timeout` on every connection so the
/// flush-loop connection and command-path connections can write concurrently
/// without spurious `SQLITE_BUSY` errors.
pub fn open_retention_db(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|e| format!("failed to open retention db: {e}"))?;

    conn.pragma_update(None, "busy_timeout", 5000)
        .map_err(|e| format!("failed to set busy_timeout: {e}"))?;
    set_wal_mode(&conn)?;

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

/// Build the retention `d_tag` column value for a kind:5 tombstone row.
///
/// Tombstones for all target kinds share `kind = 5` in the retention table, so
/// keying a tombstone by the bare target d-tag would collide across kinds when
/// a persona slug, team id, and agent pubkey happen to coincide — one
/// tombstone row would clobber another's pending publish. Folding the target
/// kind into the key (`"<target_kind>:<d_tag>"`) gives each its own PK row.
/// This is the retention-store key only; the published NIP-09 event still
/// carries the plain `a`-tag coordinate.
pub fn tombstone_retention_d_tag(target_kind: u32, d_tag: &str) -> String {
    format!("{target_kind}:{d_tag}")
}

/// Whether a pending row must be deferred to the next sweep because a kind:5
/// tombstone covering its coordinate failed to publish earlier in the same
/// sweep.
///
/// `get_pending_sync` orders tombstones first so a deletion always reaches the
/// relay before the replacement that supersedes it — but the flush is
/// best-effort per row, so a tombstone that fails mid-sweep would otherwise be
/// leapfrogged by its own replacement and then wipe it on the next sweep.
/// Deferring the replacement restores the ordering guarantee: next sweep the
/// `ORDER BY` puts the tombstone first again. Kind:5 rows are never deferred.
pub fn deferred_behind_failed_tombstone(
    kind: u32,
    pubkey: &str,
    d_tag: &str,
    failed_tombstones: &std::collections::HashSet<(String, String)>,
) -> bool {
    kind != 5
        && failed_tombstones.contains(&(pubkey.to_string(), tombstone_retention_d_tag(kind, d_tag)))
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

/// Outcome of an inbound retain — whether the local store now reflects the
/// inbound event, so the caller knows whether to patch `personas.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundOutcome {
    /// The inbound event was applied (no row, or it was strictly newer than a
    /// non-conflicting local row). The caller patches the local record store.
    Applied,
    /// The inbound event was NOT applied: either it is older than the retained
    /// row, or it collides at the same `created_at` with a pending local edit.
    /// The local record store is left untouched and the pending edit republishes.
    Skipped,
}

/// Retain an event arriving FROM the relay, resolving it against any local row.
///
/// Inbound events are already on the relay, so they are retained with
/// `pending_sync = 0`. The resolution is deliberately narrower than
/// [`retain_event`]'s blind newer-or-equal upsert, which would clobber a
/// pending local edit's `pending_sync` flag and silently drop its publish:
///
/// - No local row, or inbound strictly newer (`created_at >`): apply the
///   inbound event, clearing `pending_sync`. Inbound wins; a stale local edit
///   the relay already superseded stops republishing instead of looping.
/// - Equal `created_at`: skip. Nostr time is seconds-granularity, so a pending
///   local edit and an inbound event can share a timestamp; applying here would
///   clear `pending_sync` and drop the local publish. Skipping leaves the
///   pending row intact so the flush republishes and the relay resolves
///   last-writer-wins. (A re-received echo at equal time is also a no-op.)
/// - Inbound older: skip — nothing to change.
pub fn retain_inbound_event(
    conn: &Connection,
    event: &RetainedEvent,
) -> Result<InboundOutcome, String> {
    let existing = get_retained_event(conn, event.kind, &event.pubkey, &event.d_tag)?;

    let apply = match &existing {
        None => true,
        Some(row) if event.created_at > row.created_at => true,
        // Equal or older: skip. Equal time may collide with a pending local
        // edit, so we never clear its `pending_sync`; older is stale.
        Some(_) => false,
    };

    if !apply {
        return Ok(InboundOutcome::Skipped);
    }

    // Inbound is strictly newer (or there was no row): overwrite and clear
    // `pending_sync`. No upsert guard is needed — the Rust check above already
    // established that this event wins.
    conn.execute(
        "INSERT INTO persona_events (kind, pubkey, d_tag, content, created_at, raw_event, pending_sync)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)
         ON CONFLICT (kind, pubkey, d_tag) DO UPDATE SET
            content = excluded.content,
            created_at = excluded.created_at,
            raw_event = excluded.raw_event,
            pending_sync = 0",
        params![
            event.kind,
            event.pubkey,
            event.d_tag,
            event.content,
            event.created_at,
            event.raw_event,
        ],
    )
    .map_err(|e| format!("failed to retain inbound event: {e}"))?;

    Ok(InboundOutcome::Applied)
}

/// Load all retained persona events for a given pubkey.
#[cfg(test)]
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
///
/// Tombstones (kind:5) sort FIRST: a delete retained in one session and its
/// coordinate's replacement retained in a later one (B5 backfill resurrecting
/// a deleted definition) must publish in that order — the relay's a-tag
/// deletion soft-deletes every live row at the coordinate with no timestamp
/// comparison, so a tombstone published AFTER the replacement would wipe it.
/// Within each group, oldest first for the same reason.
pub fn get_pending_sync(conn: &Connection) -> Result<Vec<RetainedEvent>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT kind, pubkey, d_tag, content, created_at, raw_event, pending_sync
             FROM persona_events
             WHERE pending_sync = 1
             ORDER BY (kind != 5), created_at ASC",
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

/// Clear the `pending_sync` flag for an event the relay just confirmed.
///
/// Compare-and-clear: only clears the row if its `created_at` and `content`
/// still match what was published. A concurrent edit that upserted a newer
/// version at the same coordinate between the flush loop's read and this call
/// leaves `pending_sync` set, so the newer edit publishes on the next pass
/// instead of being silently dropped.
pub fn mark_synced(
    conn: &Connection,
    kind: u32,
    pubkey: &str,
    d_tag: &str,
    created_at: i64,
    content: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE persona_events SET pending_sync = 0
         WHERE kind = ?1 AND pubkey = ?2 AND d_tag = ?3
           AND created_at = ?4 AND content = ?5",
        params![kind, pubkey, d_tag, created_at, content],
    )
    .map_err(|e| format!("failed to mark event synced: {e}"))?;

    Ok(())
}

/// Delete a retained event by its coordinate.
///
/// Called from the synchronous, lock-held delete-persona command body so the
/// purge serializes against `retain_event` upserts at the same coordinate —
/// closing the same-second resurrect race where a pending edit would otherwise
/// publish after the deletion tombstone.
pub fn delete_retained_event(
    conn: &Connection,
    kind: u32,
    pubkey: &str,
    d_tag: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM persona_events
         WHERE kind = ?1 AND pubkey = ?2 AND d_tag = ?3",
        params![kind, pubkey, d_tag],
    )
    .map_err(|e| format!("failed to delete retained event: {e}"))?;

    Ok(())
}

/// Check if the retention store has any persona events for the given pubkey.
#[cfg(test)]
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

    #[test]
    fn concurrent_open_waits_for_initialization_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("retention.db");
        let first = open_retention_db(&path).unwrap();
        first.execute_batch("BEGIN EXCLUSIVE").unwrap();

        let second_path = path.clone();
        let second = std::thread::spawn(move || open_retention_db(&second_path));
        std::thread::sleep(std::time::Duration::from_millis(100));
        first.execute_batch("COMMIT").unwrap();

        assert!(second.join().unwrap().is_ok());
    }

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
    fn tombstone_retention_keys_are_distinct_across_kinds() {
        // A persona slug, team id, and agent pubkey that all happen to equal
        // "shared" must occupy DISTINCT kind:5 rows so one tombstone's pending
        // publish never clobbers another's (F2c).
        let conn = test_db();
        for target_kind in [30175u32, 30176, 30177] {
            retain_event(
                &conn,
                &RetainedEvent {
                    kind: 5,
                    pubkey: "owner".to_string(),
                    d_tag: tombstone_retention_d_tag(target_kind, "shared"),
                    content: String::new(),
                    created_at: 1000,
                    raw_event: format!("{{\"k\":{target_kind}}}"),
                    pending_sync: true,
                },
            )
            .unwrap();
        }
        // Three distinct rows survive — no PK collision clobbered any of them.
        for target_kind in [30175u32, 30176, 30177] {
            let row = get_retained_event(
                &conn,
                5,
                "owner",
                &tombstone_retention_d_tag(target_kind, "shared"),
            )
            .unwrap();
            assert!(
                row.is_some(),
                "tombstone for kind {target_kind} was clobbered"
            );
        }
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
    fn test_mark_synced_matching_row_clears_flag() {
        let conn = test_db();
        let event = sample_event();
        retain_event(&conn, &event).unwrap();

        mark_synced(&conn, 30175, "abc123", "test-persona", 1000, &event.content).unwrap();

        let pending = get_pending_sync(&conn).unwrap();
        assert!(pending.is_empty());

        let results = get_retained_personas(&conn, "abc123").unwrap();
        assert_eq!(results.len(), 1);
        assert!(!results[0].pending_sync);
    }

    #[test]
    fn test_mark_synced_stale_version_leaves_flag_set() {
        let conn = test_db();
        let published = sample_event();
        retain_event(&conn, &published).unwrap();

        // A newer edit lands at the same coordinate before the flush loop
        // clears the version it published.
        let mut newer = sample_event();
        newer.content = r#"{"display_name":"Edited"}"#.to_string();
        newer.created_at = 2000;
        retain_event(&conn, &newer).unwrap();

        // Clearing against the OLD version must not touch the newer pending row.
        mark_synced(
            &conn,
            30175,
            "abc123",
            "test-persona",
            1000,
            &published.content,
        )
        .unwrap();

        let pending = get_pending_sync(&conn).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].created_at, 2000);
    }

    #[test]
    fn test_delete_retained_event_removes_row() {
        let conn = test_db();
        retain_event(&conn, &sample_event()).unwrap();

        delete_retained_event(&conn, 30175, "abc123", "test-persona").unwrap();

        assert!(get_retained_event(&conn, 30175, "abc123", "test-persona")
            .unwrap()
            .is_none());
    }

    #[test]
    fn test_delete_retained_event_missing_row_is_noop() {
        let conn = test_db();
        delete_retained_event(&conn, 30175, "abc123", "nonexistent").unwrap();
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

    #[test]
    fn inbound_no_local_row_applies() {
        let conn = test_db();
        let mut event = sample_event();
        event.pending_sync = false;

        assert_eq!(
            retain_inbound_event(&conn, &event).unwrap(),
            InboundOutcome::Applied
        );

        let row = get_retained_event(&conn, 30175, "abc123", "test-persona")
            .unwrap()
            .unwrap();
        assert_eq!(row.created_at, 1000);
        assert!(!row.pending_sync);
    }

    #[test]
    fn inbound_equal_second_skips_and_preserves_pending() {
        let conn = test_db();
        // Pending local edit at t=1000.
        let local = sample_event();
        retain_event(&conn, &local).unwrap();

        // Inbound at the SAME second with different content.
        let inbound = RetainedEvent {
            content: r#"{"display_name":"Remote"}"#.to_string(),
            pending_sync: false,
            ..sample_event()
        };
        assert_eq!(
            retain_inbound_event(&conn, &inbound).unwrap(),
            InboundOutcome::Skipped
        );

        // Local pending row is untouched: flag preserved, content unchanged so
        // the flush republishes and the relay resolves last-writer-wins.
        let row = get_retained_event(&conn, 30175, "abc123", "test-persona")
            .unwrap()
            .unwrap();
        assert!(row.pending_sync);
        assert!(row.content.contains("Test"));
    }

    #[test]
    fn inbound_strictly_newer_applies_and_clears_pending() {
        let conn = test_db();
        // Pending local edit at t=1000.
        let local = sample_event();
        retain_event(&conn, &local).unwrap();

        // Inbound strictly newer with different content.
        let inbound = RetainedEvent {
            content: r#"{"display_name":"Remote"}"#.to_string(),
            created_at: 2000,
            pending_sync: false,
            ..sample_event()
        };
        assert_eq!(
            retain_inbound_event(&conn, &inbound).unwrap(),
            InboundOutcome::Applied
        );

        // Inbound wins: content replaced and pending cleared, so the stale
        // local edit stops republishing instead of looping.
        let row = get_retained_event(&conn, 30175, "abc123", "test-persona")
            .unwrap()
            .unwrap();
        assert_eq!(row.created_at, 2000);
        assert!(!row.pending_sync);
        assert!(row.content.contains("Remote"));
    }

    #[test]
    fn inbound_older_skips() {
        let conn = test_db();
        let mut local = sample_event();
        local.created_at = 2000;
        retain_event(&conn, &local).unwrap();

        let inbound = RetainedEvent {
            content: r#"{"display_name":"Stale"}"#.to_string(),
            created_at: 1000,
            pending_sync: false,
            ..sample_event()
        };
        assert_eq!(
            retain_inbound_event(&conn, &inbound).unwrap(),
            InboundOutcome::Skipped
        );

        let row = get_retained_event(&conn, 30175, "abc123", "test-persona")
            .unwrap()
            .unwrap();
        assert_eq!(row.created_at, 2000);
        assert!(!row.content.contains("Stale"));
    }

    #[test]
    fn pending_sync_publishes_tombstones_before_replacements() {
        // B5 resurrection race: a kind:5 retained in session N and the same
        // coordinate's replacement 30175 retained on the next boot can sit
        // pending together. The relay's a-tag deletion ignores timestamps,
        // so the tombstone MUST publish first or it wipes the replacement.
        let conn = test_db();
        let replacement = RetainedEvent {
            kind: 30175,
            created_at: 2000,
            pending_sync: true,
            ..sample_event()
        };
        retain_event(&conn, &replacement).unwrap();
        let tombstone = RetainedEvent {
            kind: 5,
            d_tag: tombstone_retention_d_tag(30175, "test-persona"),
            content: String::new(),
            created_at: 1000,
            pending_sync: true,
            ..sample_event()
        };
        retain_event(&conn, &tombstone).unwrap();

        let pending = get_pending_sync(&conn).unwrap();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].kind, 5, "tombstone first");
        assert_eq!(pending[1].kind, 30175, "replacement second");
    }

    #[test]
    fn deferral_predicate_is_kind_and_pubkey_qualified() {
        // Mid-sweep barrier semantics: a failed tombstone defers ONLY the
        // replacement at its exact coordinate — same target kind, same pubkey.
        use std::collections::HashSet;

        let failed: HashSet<(String, String)> = HashSet::from([(
            "abc123".to_string(),
            tombstone_retention_d_tag(30175, "test-persona"),
        )]);

        // The covered replacement defers.
        assert!(deferred_behind_failed_tombstone(
            30175,
            "abc123",
            "test-persona",
            &failed
        ));
        // Kind-qualified: a coinciding slug under a DIFFERENT kind is a
        // distinct coordinate (the cross-kind collision the retention d-tag
        // encoding exists to prevent) — never deferred.
        assert!(!deferred_behind_failed_tombstone(
            30177,
            "abc123",
            "test-persona",
            &failed
        ));
        // Never crosses pubkeys.
        assert!(!deferred_behind_failed_tombstone(
            30175,
            "other-key",
            "test-persona",
            &failed
        ));
        // Never defers kind:5 rows, even at a "matching" retention key.
        assert!(!deferred_behind_failed_tombstone(
            5,
            "abc123",
            "test-persona",
            &failed
        ));
        // Unrelated d-tags publish normally.
        assert!(!deferred_behind_failed_tombstone(
            30175,
            "abc123",
            "other-persona",
            &failed
        ));
    }
}
