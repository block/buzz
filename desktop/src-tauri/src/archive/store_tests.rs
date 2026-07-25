//! Unit tests for `archive/store.rs`.
//!
//! Kept in a sibling file so `store.rs` stays under the 1000-line gate;
//! `#[path]`-included from there.

use super::*;

fn in_memory() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn.pragma_update(None, "busy_timeout", 5000).unwrap();
    conn.execute_batch(SCHEMA).unwrap();
    conn
}

// ── Schema init ──────────────────────────────────────────────────────────

#[test]
fn test_schema_init_creates_all_tables() {
    let conn = in_memory();
    // Verify all three tables exist by inserting a row in each.
    conn.execute(
        "INSERT INTO save_subscriptions VALUES ('pk','relay','channel_h','abc','[1]',0)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO archived_events VALUES ('pk','relay','id1',1,'author',0,'{}',0)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO archived_event_scopes VALUES ('pk','relay','id1','channel_h','abc',0)",
        [],
    )
    .unwrap();
}

#[test]
fn test_schema_init_is_idempotent() {
    // Running SCHEMA twice must not error.
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(SCHEMA).unwrap();
    conn.execute_batch(SCHEMA).unwrap();
}

// ── Save subscriptions ───────────────────────────────────────────────────

#[test]
fn test_upsert_save_subscription_inserts_and_updates_kinds() {
    let conn = in_memory();
    upsert_save_subscription(&conn, "pk", "wss://r", "channel_h", "abc", "[1]", 1).unwrap();
    let subs = list_save_subscriptions(&conn, "pk", "wss://r").unwrap();
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].kinds, "[1]");

    // Update kinds.
    upsert_save_subscription(&conn, "pk", "wss://r", "channel_h", "abc", "[1,6]", 2).unwrap();
    let subs = list_save_subscriptions(&conn, "pk", "wss://r").unwrap();
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].kinds, "[1,6]");
}

#[test]
fn test_list_save_subscriptions_scoped_to_identity_and_relay() {
    let conn = in_memory();
    upsert_save_subscription(&conn, "pk1", "wss://r1", "channel_h", "a", "[1]", 1).unwrap();
    upsert_save_subscription(&conn, "pk2", "wss://r1", "channel_h", "b", "[1]", 2).unwrap();
    upsert_save_subscription(&conn, "pk1", "wss://r2", "channel_h", "c", "[1]", 3).unwrap();

    let subs = list_save_subscriptions(&conn, "pk1", "wss://r1").unwrap();
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].scope_value, "a");
}

#[test]
fn test_delete_save_subscription_removes_row() {
    let conn = in_memory();
    upsert_save_subscription(&conn, "pk", "wss://r", "channel_h", "abc", "[1]", 1).unwrap();
    let deleted = delete_save_subscription(&conn, "pk", "wss://r", "channel_h", "abc").unwrap();
    assert!(deleted);
    let subs = list_save_subscriptions(&conn, "pk", "wss://r").unwrap();
    assert!(subs.is_empty());
}

#[test]
fn test_delete_save_subscription_returns_false_when_not_found() {
    let conn = in_memory();
    let deleted = delete_save_subscription(&conn, "pk", "wss://r", "channel_h", "nope").unwrap();
    assert!(!deleted);
}

#[test]
fn test_has_save_subscription_true_and_false() {
    let conn = in_memory();
    upsert_save_subscription(&conn, "pk", "wss://r", "owner_p", "mypk", "[24200]", 1).unwrap();
    assert!(has_save_subscription(&conn, "pk", "wss://r", "owner_p", "mypk").unwrap());
    assert!(!has_save_subscription(&conn, "pk", "wss://r", "owner_p", "other").unwrap());
}

// ── merge_owner_p_kinds ──────────────────────────────────────────────────

#[test]
fn test_merge_owner_p_kinds_creates_row_when_none_exists() {
    let conn = in_memory();
    merge_owner_p_kinds(&conn, "pk", "wss://r", "mypk", 24200, 1).unwrap();
    let subs = list_save_subscriptions(&conn, "pk", "wss://r").unwrap();
    assert_eq!(subs.len(), 1);
    let kinds: Vec<u32> = serde_json::from_str(&subs[0].kinds).unwrap();
    assert_eq!(kinds, [24200]);
}

#[test]
fn test_merge_owner_p_kinds_adds_new_kind_to_existing_row() {
    let conn = in_memory();
    // Seed with 24200 first.
    merge_owner_p_kinds(&conn, "pk", "wss://r", "mypk", 24200, 1).unwrap();
    // Now merge 44200 in — must produce [24200, 44200].
    merge_owner_p_kinds(&conn, "pk", "wss://r", "mypk", 44200, 2).unwrap();
    let subs = list_save_subscriptions(&conn, "pk", "wss://r").unwrap();
    assert_eq!(subs.len(), 1);
    let kinds: Vec<u32> = serde_json::from_str(&subs[0].kinds).unwrap();
    assert!(kinds.contains(&24200), "must still contain 24200");
    assert!(kinds.contains(&44200), "must now contain 44200");
    assert_eq!(kinds.len(), 2, "no duplicates");
}

#[test]
fn test_merge_owner_p_kinds_idempotent_on_existing_kind() {
    let conn = in_memory();
    merge_owner_p_kinds(&conn, "pk", "wss://r", "mypk", 44200, 1).unwrap();
    merge_owner_p_kinds(&conn, "pk", "wss://r", "mypk", 44200, 2).unwrap();
    let subs = list_save_subscriptions(&conn, "pk", "wss://r").unwrap();
    let kinds: Vec<u32> = serde_json::from_str(&subs[0].kinds).unwrap();
    assert_eq!(kinds, [44200], "no duplicates after idempotent call");
}

/// Two-connection WAL regression test for the BEGIN IMMEDIATE fix.
///
/// Opens TWO separate connections to the same WAL file (mirroring the
/// real scenario where the observer-archive seed hook and the
/// metric-archive seed hook each open their own connection via
/// `open_archive_db`).  Both threads call `merge_owner_p_kinds`
/// concurrently from an empty row.  With `BEGIN IMMEDIATE` the losing
/// thread blocks on `busy_timeout` until the winner commits, then
/// reads the committed row and merges its kind in.  Both calls must
/// resolve `Ok` and the final row must contain exactly `[24200, 44200]`.
///
/// A `DEFERRED` transaction would produce `SQLITE_BUSY_SNAPSHOT` on the
/// loser (not retried by busy_timeout), causing one kind to be silently
/// dropped.  This test fails in <10 ms if the IMMEDIATE guard is removed.
#[test]
fn test_merge_owner_p_kinds_two_conn_wal_both_kinds_survive() {
    use std::sync::{Arc, Barrier};
    use std::thread;
    use tempfile::NamedTempFile;

    // A real file DB is required for WAL mode (in-memory dbs don't support
    // shared-cache WAL across multiple connections in the same process).
    let db_file = NamedTempFile::new().unwrap();
    let db_path = db_file.path().to_path_buf();

    // Initialise schema on the file DB via conn-A so both threads see it.
    let init_conn = open_archive_db(&db_path).unwrap();
    drop(init_conn);

    // Barrier ensures both threads are inside `merge_owner_p_kinds` before
    // either one issues `BEGIN IMMEDIATE`, maximising the race window.
    let barrier = Arc::new(Barrier::new(2));

    let path_a = db_path.clone();
    let path_b = db_path.clone();
    let bar_a = Arc::clone(&barrier);
    let bar_b = Arc::clone(&barrier);

    let handle_observer = thread::spawn(move || {
        let conn = open_archive_db(&path_a).unwrap();
        bar_a.wait(); // sync: both threads ready
        merge_owner_p_kinds(&conn, "pk", "wss://r", "mypk", 24200, 1)
    });

    let handle_metric = thread::spawn(move || {
        let conn = open_archive_db(&path_b).unwrap();
        bar_b.wait(); // sync: both threads ready
        merge_owner_p_kinds(&conn, "pk", "wss://r", "mypk", 44200, 2)
    });

    let res_observer = handle_observer.join().expect("observer thread panicked");
    let res_metric = handle_metric.join().expect("metric thread panicked");

    assert!(
        res_observer.is_ok(),
        "observer seed must succeed: {:?}",
        res_observer
    );
    assert!(
        res_metric.is_ok(),
        "metric seed must succeed: {:?}",
        res_metric
    );

    // Verify the final row contains both kinds.
    let verify_conn = open_archive_db(&db_path).unwrap();
    let subs = list_save_subscriptions(&verify_conn, "pk", "wss://r").unwrap();
    assert_eq!(subs.len(), 1, "exactly one owner_p row");
    let kinds: Vec<u32> = serde_json::from_str(&subs[0].kinds).unwrap();
    assert!(
        kinds.contains(&24200),
        "observer kind 24200 must survive concurrent metric seed; got {:?}",
        kinds
    );
    assert!(
        kinds.contains(&44200),
        "metric kind 44200 must be present after concurrent seed; got {:?}",
        kinds
    );
    assert_eq!(
        kinds.len(),
        2,
        "exactly two kinds, no duplicates; got {:?}",
        kinds
    );
}

// ── remove_owner_p_kind ──────────────────────────────────────────────────

#[test]
fn test_remove_owner_p_kind_removes_one_kind_leaving_other() {
    let conn = in_memory();
    // Seed both kinds.
    merge_owner_p_kinds(&conn, "pk", "wss://r", "mypk", 24200, 1).unwrap();
    merge_owner_p_kinds(&conn, "pk", "wss://r", "mypk", 44200, 2).unwrap();

    // Remove 44200 — 24200 must survive.
    remove_owner_p_kind(&conn, "pk", "wss://r", "mypk", 44200).unwrap();

    let subs = list_save_subscriptions(&conn, "pk", "wss://r").unwrap();
    assert_eq!(subs.len(), 1, "row must still exist");
    let kinds: Vec<u32> = serde_json::from_str(&subs[0].kinds).unwrap();
    assert_eq!(
        kinds,
        [24200],
        "only 24200 remains after removing 44200; got {kinds:?}"
    );
}

#[test]
fn test_remove_owner_p_kind_deletes_row_when_last_kind_removed() {
    let conn = in_memory();
    merge_owner_p_kinds(&conn, "pk", "wss://r", "mypk", 44200, 1).unwrap();

    // Remove the only kind — row must be deleted.
    remove_owner_p_kind(&conn, "pk", "wss://r", "mypk", 44200).unwrap();

    let subs = list_save_subscriptions(&conn, "pk", "wss://r").unwrap();
    assert!(
        subs.is_empty(),
        "row must be deleted when last kind removed"
    );
}

#[test]
fn test_remove_owner_p_kind_noop_when_row_absent() {
    let conn = in_memory();
    // No row exists — must succeed silently.
    remove_owner_p_kind(&conn, "pk", "wss://r", "mypk", 24200).unwrap();
    let subs = list_save_subscriptions(&conn, "pk", "wss://r").unwrap();
    assert!(subs.is_empty());
}

#[test]
fn test_remove_owner_p_kind_noop_when_kind_absent() {
    let conn = in_memory();
    // Row exists with only 24200 — removing 44200 must leave row unchanged.
    merge_owner_p_kinds(&conn, "pk", "wss://r", "mypk", 24200, 1).unwrap();
    remove_owner_p_kind(&conn, "pk", "wss://r", "mypk", 44200).unwrap();
    let subs = list_save_subscriptions(&conn, "pk", "wss://r").unwrap();
    assert_eq!(subs.len(), 1);
    let kinds: Vec<u32> = serde_json::from_str(&subs[0].kinds).unwrap();
    assert_eq!(kinds, [24200]);
}

// ── Archived events ──────────────────────────────────────────────────────

#[test]
fn test_upsert_archived_event_is_idempotent() {
    let conn = in_memory();
    let first =
        upsert_archived_event(&conn, "pk", "wss://r", "id1", 1, "author", 100, "{}", 200).unwrap();
    assert!(first, "first insert of a new id must report newly-inserted");
    // Second call must not error or duplicate, and must report false (no new row).
    let second =
        upsert_archived_event(&conn, "pk", "wss://r", "id1", 1, "author", 100, "{}", 201).unwrap();
    assert!(!second, "duplicate insert must report NOT newly-inserted");
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM archived_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

// ── Many-to-many scope rows ──────────────────────────────────────────────

#[test]
fn test_one_event_gets_multiple_scope_rows() {
    let conn = in_memory();
    upsert_archived_event(&conn, "pk", "wss://r", "id1", 1, "author", 100, "{}", 200).unwrap();
    upsert_event_scope(&conn, "pk", "wss://r", "id1", "channel_h", "chan1", 200).unwrap();
    upsert_event_scope(&conn, "pk", "wss://r", "id1", "referenced_e", "evref", 200).unwrap();
    // Idempotent second insert.
    upsert_event_scope(&conn, "pk", "wss://r", "id1", "channel_h", "chan1", 201).unwrap();

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM archived_event_scopes WHERE id = 'id1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 2);
}

// ── GC ───────────────────────────────────────────────────────────────────

#[test]
fn test_gc_removes_event_when_last_scope_deleted() {
    let conn = in_memory();
    upsert_archived_event(&conn, "pk", "wss://r", "id1", 1, "author", 100, "{}", 200).unwrap();
    upsert_event_scope(&conn, "pk", "wss://r", "id1", "channel_h", "c1", 200).unwrap();
    // Delete the only scope row manually.
    conn.execute("DELETE FROM archived_event_scopes WHERE id = 'id1'", [])
        .unwrap();
    let removed = gc_orphaned_events(&conn, "pk", "wss://r").unwrap();
    assert_eq!(removed, 1);
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM archived_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn test_gc_leaves_event_with_remaining_scope() {
    let conn = in_memory();
    upsert_archived_event(&conn, "pk", "wss://r", "id1", 1, "author", 100, "{}", 200).unwrap();
    upsert_event_scope(&conn, "pk", "wss://r", "id1", "channel_h", "c1", 200).unwrap();
    upsert_event_scope(&conn, "pk", "wss://r", "id1", "referenced_e", "ref", 200).unwrap();
    // Delete only one scope row.
    conn.execute(
        "DELETE FROM archived_event_scopes WHERE scope_type = 'referenced_e'",
        [],
    )
    .unwrap();
    let removed = gc_orphaned_events(&conn, "pk", "wss://r").unwrap();
    assert_eq!(removed, 0);
}

// ── read_archived_events ─────────────────────────────────────────────────

fn seed_events(conn: &Connection) {
    // Three events in scope "channel_h/chan1" for identity "pk"/"wss://r".
    // created_at: 300 (newest), 200, 100 (oldest).
    for (id, kind, created_at, raw) in &[
        ("e1", 9i64, 300i64, r#"{"id":"e1","created_at":300}"#),
        ("e2", 9i64, 200i64, r#"{"id":"e2","created_at":200}"#),
        ("e3", 42i64, 100i64, r#"{"id":"e3","created_at":100}"#),
    ] {
        upsert_archived_event(
            conn,
            "pk",
            "wss://r",
            id,
            *kind,
            "author",
            *created_at,
            raw,
            999,
        )
        .unwrap();
        upsert_event_scope(conn, "pk", "wss://r", id, "channel_h", "chan1", 999).unwrap();
    }
    // One event in a different scope — must never appear in chan1 results.
    upsert_archived_event(
        conn,
        "pk",
        "wss://r",
        "e4",
        9,
        "author",
        250,
        r#"{"id":"e4"}"#,
        999,
    )
    .unwrap();
    upsert_event_scope(conn, "pk", "wss://r", "e4", "channel_h", "chan2", 999).unwrap();
}

#[test]
fn test_read_archived_events_returns_newest_first() {
    let conn = in_memory();
    seed_events(&conn);
    let rows = read_archived_events(
        &conn,
        "pk",
        "wss://r",
        "channel_h",
        "chan1",
        None,
        None,
        None,
        10,
    )
    .unwrap();
    assert_eq!(rows.len(), 3);
    // Newest first: e1 (300), e2 (200), e3 (100).
    let ids: Vec<&str> = rows
        .iter()
        .map(|r| {
            if r.contains("\"e1\"") {
                "e1"
            } else if r.contains("\"e2\"") {
                "e2"
            } else {
                "e3"
            }
        })
        .collect();
    assert_eq!(ids, ["e1", "e2", "e3"]);
}

#[test]
fn test_read_archived_events_keyset_cursor_excludes_at_boundary() {
    let conn = in_memory();
    seed_events(&conn);
    // Compound cursor at e1 (created_at=300, id="e1"): excludes e1 itself,
    // returns e2 and e3.
    let rows = read_archived_events(
        &conn,
        "pk",
        "wss://r",
        "channel_h",
        "chan1",
        None,
        Some(300),
        Some("e1"),
        10,
    )
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| !r.contains("\"e1\"")));
}

#[test]
fn test_read_archived_events_keyset_cursor_advances_correctly() {
    let conn = in_memory();
    seed_events(&conn);
    // Page 1: before=None/None, limit=2 → e1, e2.
    let page1 = read_archived_events(
        &conn,
        "pk",
        "wss://r",
        "channel_h",
        "chan1",
        None,
        None,
        None,
        2,
    )
    .unwrap();
    assert_eq!(page1.len(), 2);
    // Page 2: compound cursor at e2 (created_at=200, id="e2") → e3 only.
    let page2 = read_archived_events(
        &conn,
        "pk",
        "wss://r",
        "channel_h",
        "chan1",
        None,
        Some(200),
        Some("e2"),
        2,
    )
    .unwrap();
    assert_eq!(page2.len(), 1);
    assert!(page2[0].contains("\"e3\""));
    // No overlap between pages.
    assert!(page1.iter().all(|r| !r.contains("\"e3\"")));
}

#[test]
fn test_read_archived_events_kind_filter() {
    let conn = in_memory();
    seed_events(&conn);
    // Only kind 9 (e1 and e2); e3 is kind 42.
    let rows = read_archived_events(
        &conn,
        "pk",
        "wss://r",
        "channel_h",
        "chan1",
        Some(&[9]),
        None,
        None,
        10,
    )
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| !r.contains("\"e3\"")));
}

#[test]
fn test_read_archived_events_scope_isolation() {
    let conn = in_memory();
    seed_events(&conn);
    // chan2 has only e4; chan1 results must not include e4.
    let chan1 = read_archived_events(
        &conn,
        "pk",
        "wss://r",
        "channel_h",
        "chan1",
        None,
        None,
        None,
        10,
    )
    .unwrap();
    assert!(chan1.iter().all(|r| !r.contains("\"e4\"")));

    let chan2 = read_archived_events(
        &conn,
        "pk",
        "wss://r",
        "channel_h",
        "chan2",
        None,
        None,
        None,
        10,
    )
    .unwrap();
    assert_eq!(chan2.len(), 1);
    assert!(chan2[0].contains("\"e4\""));
}

#[test]
fn test_read_archived_events_identity_isolation() {
    let conn = in_memory();
    seed_events(&conn);
    // Different identity — must see no rows.
    let rows = read_archived_events(
        &conn,
        "pk2",
        "wss://r",
        "channel_h",
        "chan1",
        None,
        None,
        None,
        10,
    )
    .unwrap();
    assert!(rows.is_empty());
}

#[test]
fn test_read_archived_events_relay_isolation() {
    let conn = in_memory();
    seed_events(&conn);
    // Different relay — must see no rows.
    let rows = read_archived_events(
        &conn,
        "pk",
        "wss://other",
        "channel_h",
        "chan1",
        None,
        None,
        None,
        10,
    )
    .unwrap();
    assert!(rows.is_empty());
}

#[test]
fn test_read_archived_events_empty_result() {
    let conn = in_memory();
    let rows = read_archived_events(
        &conn,
        "pk",
        "wss://r",
        "channel_h",
        "nope",
        None,
        None,
        None,
        10,
    )
    .unwrap();
    assert!(rows.is_empty());
}

#[test]
fn test_read_archived_events_limit_respected() {
    let conn = in_memory();
    seed_events(&conn);
    let rows = read_archived_events(
        &conn,
        "pk",
        "wss://r",
        "channel_h",
        "chan1",
        None,
        None,
        None,
        1,
    )
    .unwrap();
    assert_eq!(rows.len(), 1);
    // Must be the newest (e1, created_at=300).
    assert!(rows[0].contains("\"e1\""));
}

#[test]
fn test_read_archived_events_no_duplicates_across_pages() {
    let conn = in_memory();
    seed_events(&conn);
    let page1 = read_archived_events(
        &conn,
        "pk",
        "wss://r",
        "channel_h",
        "chan1",
        None,
        None,
        None,
        2,
    )
    .unwrap();
    // Compound cursor at e2 (the oldest in page1: created_at=200, id="e2").
    let page2 = read_archived_events(
        &conn,
        "pk",
        "wss://r",
        "channel_h",
        "chan1",
        None,
        Some(200),
        Some("e2"),
        2,
    )
    .unwrap();
    // All event ids across both pages are unique.
    let all: Vec<_> = page1.iter().chain(page2.iter()).collect();
    assert_eq!(all.len(), 3); // 2 + 1 = 3 total, no duplication.
}

// ── Migration M1: harness column + open_archive_db reopen tests ──────────────

/// Build a DB file that looks like a pre-harness archive: schema without the
/// `harness` column, a seeded `archived_events` row and a corresponding
/// `agent_metric_index` row with `harness = NULL`, and no `archive_migrations`
/// marker.  Return the path to the temp file.
fn build_old_schema_db(path: &std::path::Path) {
    // Old schema has no `harness TEXT` column and no `archive_migrations` table.
    const OLD_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS archived_events (
    identity_pubkey TEXT NOT NULL, relay_url TEXT NOT NULL, id TEXT NOT NULL,
    kind INTEGER NOT NULL, pubkey TEXT NOT NULL, created_at INTEGER NOT NULL,
    raw_json TEXT NOT NULL, archived_at INTEGER NOT NULL,
    PRIMARY KEY (identity_pubkey, relay_url, id)
);
CREATE TABLE IF NOT EXISTS archived_event_scopes (
    identity_pubkey TEXT NOT NULL, relay_url TEXT NOT NULL, id TEXT NOT NULL,
    scope_type TEXT NOT NULL, scope_value TEXT NOT NULL, archived_at INTEGER NOT NULL,
    PRIMARY KEY (identity_pubkey, relay_url, id, scope_type, scope_value)
);
CREATE TABLE IF NOT EXISTS save_subscriptions (
    identity_pubkey TEXT NOT NULL, relay_url TEXT NOT NULL, scope_type TEXT NOT NULL,
    scope_value TEXT NOT NULL, kinds TEXT NOT NULL, created_at INTEGER NOT NULL,
    PRIMARY KEY (identity_pubkey, relay_url, scope_type, scope_value)
);
CREATE TABLE IF NOT EXISTS observer_channel_index (
    identity_pubkey TEXT NOT NULL, relay_url TEXT NOT NULL, id TEXT NOT NULL,
    channel_id TEXT, created_at INTEGER NOT NULL,
    PRIMARY KEY (identity_pubkey, relay_url, id)
);
CREATE TABLE IF NOT EXISTS agent_metric_index (
    identity_pubkey TEXT NOT NULL, relay_url TEXT NOT NULL, id TEXT NOT NULL,
    agent_pubkey TEXT NOT NULL, event_created_at INTEGER NOT NULL,
    archived_at INTEGER NOT NULL, reported_at INTEGER, session_id TEXT,
    turn_seq TEXT, model TEXT, delta_reliable INTEGER,
    turn_input_tokens TEXT, turn_output_tokens TEXT, turn_total_tokens TEXT,
    turn_cost_usd REAL, cumulative_input_tokens TEXT, cumulative_output_tokens TEXT,
    cumulative_total_tokens TEXT, cumulative_cost_usd REAL,
    parse_status TEXT NOT NULL CHECK (parse_status IN ('valid','invalid')),
    PRIMARY KEY (identity_pubkey, relay_url, id)
);
";
    let conn = Connection::open(path).unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn.pragma_update(None, "busy_timeout", 5000).unwrap();
    conn.execute_batch(OLD_SCHEMA).unwrap();

    // A valid payload that carries harness="goose".
    let raw_json = r#"{"harness":"goose","model":"claude","channelId":null,"sessionId":"s1","turnId":null,"turnSeq":1,"timestamp":"2026-07-01T00:00:00Z","turn":{"inputTokens":10,"outputTokens":20,"totalTokens":30,"costUsd":0.01},"cumulative":{"inputTokens":100,"outputTokens":200,"totalTokens":300,"costUsd":0.1},"deltaReliable":true,"stopReason":"end_turn"}"#;
    conn.execute(
        "INSERT INTO archived_events
             (identity_pubkey, relay_url, id, kind, pubkey, created_at, raw_json, archived_at)
         VALUES ('id', 'relay', 'eid1', 44200, 'agent1', 100, ?1, 200)",
        rusqlite::params![raw_json],
    )
    .unwrap();
    // Seed the index row with harness absent (old schema has no column).
    conn.execute(
        "INSERT INTO agent_metric_index
             (identity_pubkey, relay_url, id, agent_pubkey, event_created_at,
              archived_at, reported_at, session_id, turn_seq, model,
              delta_reliable, turn_input_tokens, turn_output_tokens,
              turn_total_tokens, turn_cost_usd, cumulative_input_tokens,
              cumulative_output_tokens, cumulative_total_tokens,
              cumulative_cost_usd, parse_status)
         VALUES ('id','relay','eid1','agent1',100,200,1751414400000,
                 's1','00000000000000000001','claude',
                 1,'00000000000000000010','00000000000000000020',
                 '00000000000000000030',0.01,
                 '00000000000000000100','00000000000000000200',
                 '00000000000000000300',0.1,'valid')",
        [],
    )
    .unwrap();
}

/// Opening a pre-harness DB via `open_archive_db` must:
///
/// 1. Add the `harness` column.
/// 2. Rebuild all index rows so harness is populated from `archived_events`.
/// 3. Record the migration marker in `archive_migrations`.
///
/// This proves M1 fires on a real legacy file, not just a hand-rolled
/// DELETE+backfill as in the metric_store migration test.
#[test]
fn migration_m1_reopen_old_schema_db_populates_harness_and_records_marker() {
    use tempfile::NamedTempFile;
    let db_file = NamedTempFile::new().unwrap();
    let db_path = db_file.path().to_path_buf();

    build_old_schema_db(&db_path);

    // Reopen via the real migration path.
    let conn = open_archive_db(&db_path).expect("open_archive_db must succeed on legacy DB");

    // Harness must be populated.
    let harness: Option<String> = conn
        .query_row(
            "SELECT harness FROM agent_metric_index WHERE id = 'eid1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        harness,
        Some("goose".to_string()),
        "M1: harness must be populated from raw_json after reopen"
    );

    // Marker must be recorded.
    let marker_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM archive_migrations \
             WHERE name = 'add_harness_to_metric_index'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(marker_count, 1, "M1: migration marker must be recorded");
}

/// Re-opening the same DB a second time must be a no-op: marker already present,
/// harness already populated, no error.
#[test]
fn migration_m1_reopen_twice_is_idempotent() {
    use tempfile::NamedTempFile;
    let db_file = NamedTempFile::new().unwrap();
    let db_path = db_file.path().to_path_buf();

    build_old_schema_db(&db_path);

    // First open: runs M1.
    open_archive_db(&db_path).expect("first open must succeed");
    // Second open: M1 is already marked; must also succeed without error.
    let conn2 = open_archive_db(&db_path).expect("second open must succeed (M1 is idempotent)");

    let count: i64 = conn2
        .query_row(
            "SELECT COUNT(*) FROM agent_metric_index WHERE harness = 'goose'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 1,
        "after idempotent second open, harness row must still be present"
    );
}

/// If the migration is aborted after DELETE but before COMMIT (simulated by
/// manually applying each sub-step without committing), the DB must be in its
/// pre-migration state and the marker must be absent, so the next
/// `open_archive_db` can re-run M1 from scratch and complete successfully.
///
/// We simulate the "crash before commit" scenario by:
/// 1. Building a pre-harness DB on disk.
/// 2. Opening a raw connection, running ALTER + DELETE but rolling back before
///    inserting the marker — leaving the DB in pre-migration state.
/// 3. Calling `open_archive_db` on the same file and asserting M1 completes.
#[test]
fn migration_m1_crashed_before_commit_reruns_fully_on_next_open() {
    use tempfile::NamedTempFile;
    let db_file = NamedTempFile::new().unwrap();
    let db_path = db_file.path().to_path_buf();

    build_old_schema_db(&db_path);

    // Simulate a crash: start a transaction, ALTER + DELETE, then ROLLBACK
    // (leaving DB exactly as-built — pre-migration, no marker).
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        // ALTER inside the transaction.
        conn.execute_batch("ALTER TABLE agent_metric_index ADD COLUMN harness TEXT")
            .unwrap();
        // DELETE index rows (simulating the mid-migration state).
        conn.execute_batch("DELETE FROM agent_metric_index")
            .unwrap();
        // Rollback — no marker inserted, no new rows.
        conn.execute_batch("ROLLBACK").unwrap();
    }

    // After rollback: marker must be absent and the original index row still there.
    {
        let verify = Connection::open(&db_path).unwrap();
        // archive_migrations table may not exist at all (old schema). If it does,
        // the marker must not be present.
        let marker_count: i64 = verify
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type='table' AND name='archive_migrations'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        if marker_count > 0 {
            let applied: i64 = verify
                .query_row(
                    "SELECT COUNT(*) FROM archive_migrations \
                     WHERE name = 'add_harness_to_metric_index'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(applied, 0, "marker must be absent after rollback");
        }
    }

    // The next open must run M1 fully and succeed.
    let conn =
        open_archive_db(&db_path).expect("open_archive_db must succeed after simulated crash");

    // Harness must be populated.
    let harness: Option<String> = conn
        .query_row(
            "SELECT harness FROM agent_metric_index WHERE id = 'eid1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        harness,
        Some("goose".to_string()),
        "M1 must re-run after simulated crash and populate harness"
    );

    // Marker must now be recorded.
    let marker: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM archive_migrations \
             WHERE name = 'add_harness_to_metric_index'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        marker, 1,
        "M1 marker must be recorded after successful re-run"
    );
}

/// Regression for the scalar-cursor same-second skip defect (Thufir IMPORTANT).
///
/// The writer stores `created_at` in whole seconds, so two events can share
/// the same timestamp.  The sort order is `(created_at DESC, id DESC)`, so
/// a page split exactly at a same-second boundary leaves one sibling on each
/// side.  With only `created_at < before` the second-page sibling would be
/// permanently excluded.  The compound `(created_at < ?) OR (created_at = ?
/// AND id < ?)` predicate mirrors the sort key exactly and avoids the skip.
#[test]
fn test_read_archived_events_same_second_cursor_no_skip() {
    let conn = in_memory();
    // Two events share created_at=1000. Sort order: "z" (id "z") > "a" (id "a"),
    // so ORDER BY created_at DESC, id DESC yields: ("z", 1000) first, ("a", 1000) second.
    // A third event has created_at=500.
    for (id, kind, created_at, raw) in &[
        ("z", 9i64, 1000i64, r#"{"id":"z","created_at":1000}"#),
        ("a", 9i64, 1000i64, r#"{"id":"a","created_at":1000}"#),
        ("old", 9i64, 500i64, r#"{"id":"old","created_at":500}"#),
    ] {
        upsert_archived_event(
            &conn,
            "pk",
            "wss://r",
            id,
            *kind,
            "author",
            *created_at,
            raw,
            999,
        )
        .unwrap();
        upsert_event_scope(&conn, "pk", "wss://r", id, "channel_h", "same_sec", 999).unwrap();
    }

    // Page 1: limit=1 → should return ("z", 1000) only (newest by compound sort).
    let page1 = read_archived_events(
        &conn,
        "pk",
        "wss://r",
        "channel_h",
        "same_sec",
        None,
        None,
        None,
        1,
    )
    .unwrap();
    assert_eq!(page1.len(), 1);
    assert!(page1[0].contains("\"z\""), "page1 must be the 'z' row");

    // Page 2: compound cursor at ("z", 1000).
    // With a scalar cursor (created_at < 1000), row "a" would be SKIPPED.
    // With the compound cursor, "a" must appear on page 2.
    let page2 = read_archived_events(
        &conn,
        "pk",
        "wss://r",
        "channel_h",
        "same_sec",
        None,
        Some(1000),
        Some("z"),
        2,
    )
    .unwrap();
    // Must contain "a" (same-second sibling) and "old" (strictly older).
    assert_eq!(page2.len(), 2, "page2 must return both remaining rows");
    assert!(
        page2.iter().any(|r| r.contains("\"a\"")),
        "same-second sibling 'a' must not be skipped"
    );
    assert!(
        page2.iter().any(|r| r.contains("\"old\"")),
        "'old' row must appear on page2"
    );
    // No overlap with page1.
    assert!(page2.iter().all(|r| !r.contains("\"z\"")));
}

// ── observer_channel_index ───────────────────────────────────────────────────

#[test]
fn test_upsert_observer_channel_index_non_null_is_idempotent() {
    let conn = in_memory();
    let pk = "owner";
    let relay = "wss://r";

    upsert_observer_channel_index(&conn, pk, relay, "ev1", Some("ch-abc"), 1000).unwrap();
    // Second call with same PK → INSERT OR IGNORE, no error, row unchanged.
    upsert_observer_channel_index(&conn, pk, relay, "ev1", Some("ch-abc"), 2000).unwrap();

    // Only one row should exist and created_at stays at 1000 (first write wins).
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM observer_channel_index WHERE id = 'ev1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "INSERT OR IGNORE must not duplicate rows");

    let at: i64 = conn
        .query_row(
            "SELECT created_at FROM observer_channel_index WHERE id = 'ev1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(at, 1000, "first write's created_at must be preserved");
}

#[test]
fn test_upsert_observer_channel_index_null_channel_id_is_idempotent() {
    let conn = in_memory();
    let pk = "owner";
    let relay = "wss://r";

    // Write a NULL channel_id status row (unscoped / decrypt-failed frame).
    upsert_observer_channel_index(&conn, pk, relay, "ev-null", None, 500).unwrap();
    // Re-run → no-op.
    upsert_observer_channel_index(&conn, pk, relay, "ev-null", None, 600).unwrap();

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM observer_channel_index WHERE id = 'ev-null'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 1,
        "null-channel_id row must not be duplicated on re-run"
    );

    let channel_id: Option<String> = conn
        .query_row(
            "SELECT channel_id FROM observer_channel_index WHERE id = 'ev-null'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        channel_id.is_none(),
        "channel_id must be NULL for unscoped frames"
    );
}

#[test]
fn test_read_archived_observer_excludes_null_channel_id_rows() {
    // Scoped reads filter channel_id = '?'; NULL rows must never appear.
    let conn = in_memory();
    let pk = "owner";
    let relay = "wss://r";

    // Seed an archived event row.
    upsert_archived_event(&conn, pk, relay, "ev-null", 24200, "agent", 1000, "{}", 0).unwrap();
    upsert_event_scope(&conn, pk, relay, "ev-null", "owner_p", pk, 0).unwrap();

    // Index it with a NULL channel_id.
    upsert_observer_channel_index(&conn, pk, relay, "ev-null", None, 1000).unwrap();

    // Scoped read for ANY channel must return nothing (NULL != 'some-channel').
    let rows =
        read_archived_observer_events_for_channel(&conn, pk, relay, "some-channel", None, None, 50)
            .unwrap();
    assert!(
        rows.is_empty(),
        "scoped read must not return null-channel_id rows"
    );
}

#[test]
fn test_read_unindexed_observer_rows_excludes_processed_rows() {
    // After a NULL channel_id row is written, the event must no longer appear
    // in read_unindexed_observer_rows (it is "processed").
    let conn = in_memory();
    let pk = "owner";
    let relay = "wss://r";

    // Seed an archived observer event with owner_p scope.
    upsert_archived_event(&conn, pk, relay, "ev-old", 24200, "agent", 1000, "{}", 0).unwrap();
    upsert_event_scope(&conn, pk, relay, "ev-old", "owner_p", pk, 0).unwrap();

    // Before indexing: ev-old must appear in unindexed rows.
    let unindexed = read_unindexed_observer_rows(&conn, pk, relay).unwrap();
    assert!(
        unindexed.iter().any(|(id, _, _)| id == "ev-old"),
        "ev-old must appear in unindexed rows before indexing"
    );

    // Index it with NULL channel_id (decrypt-failed / unscoped).
    upsert_observer_channel_index(&conn, pk, relay, "ev-old", None, 1000).unwrap();

    // After indexing: ev-old must NOT appear in unindexed rows (re-run is a no-op).
    let unindexed2 = read_unindexed_observer_rows(&conn, pk, relay).unwrap();
    assert!(
        !unindexed2.iter().any(|(id, _, _)| id == "ev-old"),
        "ev-old must be excluded from unindexed rows after null-channel_id indexing"
    );
}
