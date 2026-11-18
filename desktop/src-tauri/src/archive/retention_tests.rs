//! Behavior tests for the observer-retention setting, the size readout, and the
//! M4 migration.
//!
//! Kept in a sibling file so `retention.rs` stays under the 1500-line gate;
//! `#[path]`-included from there. `super::*` brings the retention API (and its
//! `rusqlite::{params, Connection}` imports) into scope; `super::super::store`
//! reaches the neighbouring subscription mutators and the base `SCHEMA`.

use super::super::store;
use super::*;
use std::path::Path;
use std::sync::{Arc, Barrier};
use tempfile::NamedTempFile;

const ID: &str = "idpk";
const RELAY: &str = "wss://r";
const OWNER: &str = "owner_p";

/// Open a fresh archive DB (runs the full schema + every migration incl. M4).
fn fresh(db: &NamedTempFile) -> Connection {
    store::open_archive_db(db.path()).expect("open_archive_db must succeed")
}

/// Build a legacy DB that has the base schema and M1–M3 markers but NOT M4,
/// so the next `open_archive_db` pends only the retention migration. This
/// isolates the M4 first-open race from the separately-tested M1–M3 chain.
fn build_pre_m4_db(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.pragma_update(None, "busy_timeout", 5000).unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn.execute_batch(store::SCHEMA).unwrap();
    for name in [
        "add_harness_to_metric_index",
        "add_cache_read_tokens",
        "add_cache_write_and_pricing",
    ] {
        conn.execute(
            "INSERT OR IGNORE INTO archive_migrations (name, applied_at) VALUES (?1, 0)",
            params![name],
        )
        .unwrap();
    }
}

fn m4_marker_count(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM archive_migrations WHERE name = 'add_archive_meta'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

// ── Validation (pure) ──────────────────────────────────────────────────────────

#[test]
fn test_validate_days_accepts_one_and_max() {
    assert!(validate_days(1).is_ok());
    assert!(validate_days(DEFAULT_OBSERVER_RETENTION_DAYS).is_ok());
    assert!(validate_days(MAX_RETENTION_DAYS).is_ok());
}

#[test]
fn test_validate_days_rejects_zero_negative_and_over_max() {
    assert!(validate_days(0).is_err());
    assert!(validate_days(-1).is_err());
    assert!(validate_days(MAX_RETENTION_DAYS + 1).is_err());
}

// ── Observer retention get / set ────────────────────────────────────────────────

#[test]
fn test_get_observer_days_returns_seeded_default_on_fresh_db() {
    let db = NamedTempFile::new().unwrap();
    let conn = fresh(&db);
    assert_eq!(
        get_observer_retention_days(&conn).unwrap(),
        DEFAULT_OBSERVER_RETENTION_DAYS
    );
}

#[test]
fn test_set_observer_days_upserts_and_overwrites() {
    let db = NamedTempFile::new().unwrap();
    let conn = fresh(&db);
    set_observer_retention_days(&conn, 14).unwrap();
    assert_eq!(get_observer_retention_days(&conn).unwrap(), 14);
    set_observer_retention_days(&conn, 60).unwrap();
    assert_eq!(get_observer_retention_days(&conn).unwrap(), 60);
}

#[test]
fn test_set_observer_days_rejects_out_of_range_without_writing() {
    let db = NamedTempFile::new().unwrap();
    let conn = fresh(&db);
    // Establish a known good value, then prove a rejected write leaves it.
    set_observer_retention_days(&conn, 45).unwrap();
    assert!(set_observer_retention_days(&conn, 0).is_err());
    assert!(set_observer_retention_days(&conn, -5).is_err());
    assert!(set_observer_retention_days(&conn, MAX_RETENTION_DAYS + 1).is_err());
    assert_eq!(
        get_observer_retention_days(&conn).unwrap(),
        45,
        "a rejected set must not overwrite the stored value"
    );
}

#[test]
fn test_observer_days_survive_reopen() {
    let db = NamedTempFile::new().unwrap();
    {
        let first = fresh(&db);
        set_observer_retention_days(&first, 7).unwrap();
    }
    let second = fresh(&db);
    assert_eq!(
        get_observer_retention_days(&second).unwrap(),
        7,
        "the setting persists across opens and M4 re-run does not reset it"
    );
}

// ── Size accounting ─────────────────────────────────────────────────────────────

#[test]
fn test_size_stats_reports_pages_and_main_file_bytes() {
    let db = NamedTempFile::new().unwrap();
    let conn = fresh(&db);
    // Write enough rows that the DB grows past a single page.
    for i in 0..200 {
        conn.execute(
            "INSERT INTO archived_events
                 (identity_pubkey, relay_url, id, kind, pubkey, created_at, raw_json, archived_at)
             VALUES (?1, ?2, ?3, 24200, 'author', ?4, ?5, ?4)",
            params![ID, RELAY, format!("e{i}"), 1000 + i, "x".repeat(256)],
        )
        .unwrap();
    }
    let stats = size_stats(&conn).unwrap();
    assert!(stats.page_size > 0, "page_size is a positive PRAGMA value");
    assert!(stats.page_count > 1, "multi-page DB after 200 inserts");
    assert!(stats.freelist_count >= 0);
    // In WAL mode the just-written pages live in the `-wal` sidecar until a
    // checkpoint folds them into the main file, so the logical page total
    // (page_size * page_count) is covered by the two files combined, not by
    // the main file alone.
    assert!(
        stats.main_file_bytes + stats.wal_file_bytes >= stats.page_size * stats.page_count,
        "main + wal bytes cover at least the counted pages ({} + {} >= {}*{})",
        stats.main_file_bytes,
        stats.wal_file_bytes,
        stats.page_size,
        stats.page_count
    );
    assert!(stats.main_file_bytes > 0, "the main DB file is on disk");
    // WAL mode: the sidecar exists and carries the just-written frames.
    assert!(
        stats.wal_file_bytes > 0,
        "the -wal sidecar is measured in WAL mode"
    );
}

#[test]
fn test_size_stats_freelist_grows_after_delete() {
    let db = NamedTempFile::new().unwrap();
    let conn = fresh(&db);
    for i in 0..200 {
        conn.execute(
            "INSERT INTO archived_events
                 (identity_pubkey, relay_url, id, kind, pubkey, created_at, raw_json, archived_at)
             VALUES (?1, ?2, ?3, 24200, 'author', ?4, ?5, ?4)",
            params![ID, RELAY, format!("e{i}"), 1000 + i, "x".repeat(256)],
        )
        .unwrap();
    }
    let before = size_stats(&conn).unwrap();
    conn.execute("DELETE FROM archived_events", []).unwrap();
    let after = size_stats(&conn).unwrap();
    assert!(
        after.freelist_count > before.freelist_count,
        "deleted pages land on the freelist ({} > {})",
        after.freelist_count,
        before.freelist_count
    );
}

// ── Migration M4 ──────────────────────────────────────────────────────────────

#[test]
fn test_m4_fresh_open_creates_schema_marker_and_seed() {
    let db = NamedTempFile::new().unwrap();
    let conn = fresh(&db);
    let objects: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE (type = 'table' AND name = 'archive_meta')
                OR (type = 'index' AND name = 'idx_archived_event_scopes_age')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(objects, 2, "one table + one index");
    assert_eq!(m4_marker_count(&conn), 1, "M4 marker recorded");
    assert_eq!(
        get_observer_retention_days(&conn).unwrap(),
        DEFAULT_OBSERVER_RETENTION_DAYS,
        "the observer window is seeded to the default"
    );
}

#[test]
fn test_m4_reopen_is_idempotent_and_preserves_setting() {
    let db = NamedTempFile::new().unwrap();
    let first = fresh(&db);
    set_observer_retention_days(&first, 90).unwrap();
    drop(first);
    let second = fresh(&db);
    assert_eq!(
        m4_marker_count(&second),
        1,
        "exactly one marker after reopen"
    );
    assert_eq!(
        get_observer_retention_days(&second).unwrap(),
        90,
        "the re-run must not re-seed over an existing value"
    );
}

#[test]
fn test_m4_applies_over_a_populated_pre_m4_db() {
    let db = NamedTempFile::new().unwrap();
    build_pre_m4_db(db.path());
    // Pre-M4 subscriptions and archived rows must survive the migration
    // untouched — M4 no longer reads or seeds from subscriptions.
    {
        let conn = Connection::open(db.path()).unwrap();
        store::upsert_save_subscription(&conn, ID, RELAY, OWNER, ID, "[24200,44200,1]", 100)
            .unwrap();
    }
    let conn = fresh(&db);
    assert_eq!(m4_marker_count(&conn), 1);
    assert_eq!(
        get_observer_retention_days(&conn).unwrap(),
        DEFAULT_OBSERVER_RETENTION_DAYS
    );
    assert_eq!(
        store::list_save_subscriptions(&conn, ID, RELAY)
            .unwrap()
            .len(),
        1,
        "the pre-existing subscription is untouched by M4"
    );
}

#[test]
fn test_m4_preexisting_archive_meta_without_marker_fails_closed() {
    let db = NamedTempFile::new().unwrap();
    build_pre_m4_db(db.path());
    // An `archive_meta` table present without the M4 marker is unreachable via
    // shipped code — M4 runs the whole body (create table, build index, seed,
    // marker) in one transactional `BEGIN IMMEDIATE`, so a crash rolls back the
    // table too. The only way to reach this state is an externally-created
    // table. M4 refuses to certify it: it fails closed and rolls back with no
    // marker rather than adopting a table it did not build. The table's shape
    // is irrelevant — its mere presence without the marker is the trigger.
    {
        let conn = Connection::open(db.path()).unwrap();
        conn.execute_batch("CREATE TABLE archive_meta (key TEXT PRIMARY KEY);")
            .unwrap();
    }
    assert!(
        store::open_archive_db(db.path()).is_err(),
        "M4 must fail closed on a pre-existing archive_meta"
    );
    let verify = Connection::open(db.path()).unwrap();
    assert_eq!(
        m4_marker_count(&verify),
        0,
        "no marker may certify an externally-created table"
    );
}

#[test]
fn test_m4_preexisting_index_under_name_is_silently_rebuilt() {
    let db = NamedTempFile::new().unwrap();
    build_pre_m4_db(db.path());
    // Unlike a table (which carries data and is fail-closed), an index carries
    // no data, so M4 drops any index sharing the name and recreates it
    // unconditionally — no shape inspection. A bogus pre-existing index under
    // the name is silently replaced with the correct one and the marker lands.
    {
        let conn = Connection::open(db.path()).unwrap();
        conn.execute_batch(&format!(
            "CREATE INDEX {SCOPE_AGE_INDEX_NAME} ON archived_event_scopes (id);"
        ))
        .unwrap();
    }
    let conn = fresh(&db);
    assert_eq!(m4_marker_count(&conn), 1, "M4 completes after the rebuild");
    // The rebuilt index has the six expected key columns in the covering order.
    let mut stmt = conn
        .prepare(&format!(
            "SELECT name FROM pragma_index_xinfo('{SCOPE_AGE_INDEX_NAME}') \
             WHERE key = 1 ORDER BY seqno"
        ))
        .unwrap();
    let keys: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        keys,
        [
            "identity_pubkey",
            "relay_url",
            "archived_at",
            "id",
            "scope_type",
            "scope_value"
        ],
        "the index was rebuilt with the age-first covering key order"
    );
}

// ── Concurrency ───────────────────────────────────────────────────────────────

#[test]
fn test_m4_two_conn_first_open_race_neither_times_out_and_marks_once() {
    use std::thread;
    let db = NamedTempFile::new().unwrap();
    let path = db.path().to_path_buf();
    build_pre_m4_db(&path);
    // A realistically-populated legacy DB: a multi-kind subscription plus
    // archived scope rows so M4's index build touches real data.
    {
        let conn = Connection::open(&path).unwrap();
        store::upsert_save_subscription(&conn, ID, RELAY, OWNER, ID, "[24200,44200,1]", 100)
            .unwrap();
        for i in 0..8 {
            conn.execute(
                "INSERT INTO archived_event_scopes
                     (identity_pubkey, relay_url, id, scope_type, scope_value, archived_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![ID, RELAY, format!("e{i}"), OWNER, ID, 1000 + i],
            )
            .unwrap();
        }
    }

    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let p = path.clone();
            let b = Arc::clone(&barrier);
            thread::spawn(move || {
                b.wait(); // maximise the first-open race window
                store::open_archive_db(&p).map(|_| ())
            })
        })
        .collect();
    for h in handles {
        assert!(
            h.join().unwrap().is_ok(),
            "both racing opens must complete within busy_timeout"
        );
    }

    let verify = store::open_archive_db(&path).unwrap();
    assert_eq!(m4_marker_count(&verify), 1, "M4 applied exactly once");
    // The seed ran once inside the winner's transaction — one meta row, default.
    let meta_rows: i64 = verify
        .query_row(
            "SELECT COUNT(*) FROM archive_meta WHERE key = ?1",
            params![OBSERVER_RETENTION_DAYS_KEY],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(meta_rows, 1, "the observer window was seeded exactly once");
    assert_eq!(
        get_observer_retention_days(&verify).unwrap(),
        DEFAULT_OBSERVER_RETENTION_DAYS
    );
}

// ── Prune-candidate access path (index shape) ──────────────────────────────────

/// The Phase-2 prune-candidate scan (per `PLANS/ARCHIVE_RETENTION_PLAN.md`
/// line 40): the single global observer window filters `identity + relay +
/// archived_at < cutoff`, joins `archived_events` for `kind = 24200`, and
/// selects the scope-row PK so the bounded `DELETE` can materialize candidates.
/// The scope side constrains neither `scope_type` nor `scope_value`.
const PRUNE_CANDIDATE_SQL: &str = "
SELECT s.id, s.scope_type, s.scope_value
FROM archived_event_scopes s
JOIN archived_events e
  ON e.identity_pubkey = s.identity_pubkey
 AND e.relay_url = s.relay_url
 AND e.id = s.id
WHERE s.identity_pubkey = ?1
  AND s.relay_url = ?2
  AND s.archived_at < ?3
  AND e.kind = 24200
LIMIT 1000
";

/// The scope-age index must let the real Phase-2 prune query range-seek
/// `archived_at` directly rather than fall back to a bare identity/relay seek
/// plus a temp b-tree. The index is this PR's deliverable, so its access path
/// is pinned here even though the prune worker lands in Phase 2.
#[test]
fn test_prune_candidate_scan_seeks_archived_at_through_the_index() {
    let db = NamedTempFile::new().unwrap();
    let conn = fresh(&db); // runs M4, creating the scope-age index

    let plan: Vec<String> = conn
        .prepare(&format!("EXPLAIN QUERY PLAN {PRUNE_CANDIDATE_SQL}"))
        .unwrap()
        .query_map(params![ID, RELAY, 0_i64], |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    let scope_step = plan
        .iter()
        .find(|d| d.contains(SCOPE_AGE_INDEX_NAME))
        .unwrap_or_else(|| panic!("prune scan must use {SCOPE_AGE_INDEX_NAME}; plan was {plan:?}"));
    // The planner range-seeks archived_at through the index (equality on the
    // two leading identity/relay keys, then the age bound) — not a bare
    // identity/relay seek that would leave age to a scan / temp b-tree.
    assert!(
        scope_step.contains("archived_at<?"),
        "scope step must range-seek archived_at, got: {scope_step}"
    );
    assert!(
        !plan.iter().any(|d| d.contains("USE TEMP B-TREE")),
        "age-first index must remove the ORDER-BY temp b-tree; plan was {plan:?}"
    );
}
