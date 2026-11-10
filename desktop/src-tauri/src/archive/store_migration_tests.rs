//! Migration tests for `archive/store.rs` — M1: harness column.
//!
//! Kept in a sibling file so `store_tests.rs` stays under the 1500-line gate;
//! `#[path]`-included from `store.rs`.

use super::*;

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

// ── Migration M2: cache_read_tokens columns ───────────────────────────────────

/// Opening a fresh DB (created by `open_archive_db`) must have both cache
/// columns present and the M2 marker recorded.
#[test]
fn migration_m2_fresh_db_has_cache_columns_and_marker() {
    use tempfile::NamedTempFile;
    let db_file = NamedTempFile::new().unwrap();
    let conn = open_archive_db(db_file.path()).expect("open_archive_db must succeed");

    // Both columns must exist (PRAGMA table_info returns one row per column).
    let columns: Vec<String> = {
        let mut stmt = conn
            .prepare("PRAGMA table_info(agent_metric_index)")
            .unwrap();
        stmt.query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    assert!(
        columns.iter().any(|c| c == "turn_cache_read_tokens"),
        "turn_cache_read_tokens must exist on a fresh DB"
    );
    assert!(
        columns.iter().any(|c| c == "cumulative_cache_read_tokens"),
        "cumulative_cache_read_tokens must exist on a fresh DB"
    );

    let marker_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM archive_migrations WHERE name = 'add_cache_read_tokens'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(marker_count, 1, "M2 marker must be recorded on fresh DB");
}

/// Opening a post-M1, pre-M2 DB (i.e., has `harness` column and M1 marker,
/// but no `turn_cache_read_tokens` column) must add both cache columns,
/// leaving all pre-existing rows with NULL for those columns, and record the
/// M2 marker.
#[test]
fn migration_m2_upgrades_post_m1_db_and_leaves_nulls() {
    use tempfile::NamedTempFile;
    let db_file = NamedTempFile::new().unwrap();
    let db_path = db_file.path().to_path_buf();

    // Build a DB that looks like it ran M1 but not M2: the schema has
    // `harness` but not the cache columns, and M1 marker is present.
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.pragma_update(None, "busy_timeout", 5000).unwrap();
        // Use M1-era schema: harness present, no cache columns, no archive_migrations table yet.
        const M1_SCHEMA: &str = "
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
    harness TEXT,
    parse_status TEXT NOT NULL CHECK (parse_status IN ('valid','invalid')),
    PRIMARY KEY (identity_pubkey, relay_url, id)
);
CREATE TABLE IF NOT EXISTS archive_migrations (
    name TEXT PRIMARY KEY,
    applied_at INTEGER NOT NULL
);
";
        conn.execute_batch(M1_SCHEMA).unwrap();
        // Mark M1 as already run so `open_archive_db` skips it.
        conn.execute(
            "INSERT INTO archive_migrations (name, applied_at) VALUES ('add_harness_to_metric_index', 1)",
            [],
        )
        .unwrap();
        // Seed one existing index row — it should get NULL cache columns.
        let raw_json = r#"{"harness":"goose","model":"claude","channelId":null,"sessionId":"s1","turnId":null,"turnSeq":1,"timestamp":"2026-07-01T00:00:00Z","turn":{"inputTokens":10,"outputTokens":20,"totalTokens":30,"costUsd":0.01},"cumulative":{"inputTokens":100,"outputTokens":200,"totalTokens":300,"costUsd":0.1},"deltaReliable":true,"stopReason":"end_turn"}"#;
        conn.execute(
            "INSERT INTO archived_events
                 (identity_pubkey, relay_url, id, kind, pubkey, created_at, raw_json, archived_at)
             VALUES ('id', 'relay', 'eid1', 44200, 'agent1', 100, ?1, 200)",
            rusqlite::params![raw_json],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO agent_metric_index
                 (identity_pubkey, relay_url, id, agent_pubkey, event_created_at,
                  archived_at, reported_at, session_id, turn_seq, model,
                  delta_reliable, turn_input_tokens, turn_output_tokens,
                  turn_total_tokens, turn_cost_usd, cumulative_input_tokens,
                  cumulative_output_tokens, cumulative_total_tokens,
                  cumulative_cost_usd, harness, parse_status)
             VALUES ('id','relay','eid1','agent1',100,200,1751414400,
                     's1','00000000000000000001','claude',
                     1,'00000000000000000010','00000000000000000020',
                     '00000000000000000030',0.01,
                     '00000000000000000100','00000000000000000200',
                     '00000000000000000300',0.1,'goose','valid')",
            [],
        )
        .unwrap();
    }

    // Open via migration path — M1 is already marked, M2 should run.
    let conn = open_archive_db(&db_path).expect("open_archive_db must succeed on post-M1 DB");

    // Both cache columns must now exist.
    let columns: Vec<String> = {
        let mut stmt = conn
            .prepare("PRAGMA table_info(agent_metric_index)")
            .unwrap();
        stmt.query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    assert!(
        columns.iter().any(|c| c == "turn_cache_read_tokens"),
        "M2: turn_cache_read_tokens must be present after upgrade"
    );
    assert!(
        columns.iter().any(|c| c == "cumulative_cache_read_tokens"),
        "M2: cumulative_cache_read_tokens must be present after upgrade"
    );

    // Pre-existing row gets NULLs — not estimated, not backfilled.
    let (turn_cr, cum_cr): (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT turn_cache_read_tokens, cumulative_cache_read_tokens \
             FROM agent_metric_index WHERE id = 'eid1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(
        turn_cr.is_none(),
        "M2: pre-existing row turn_cache_read_tokens must be NULL"
    );
    assert!(
        cum_cr.is_none(),
        "M2: pre-existing row cumulative_cache_read_tokens must be NULL"
    );

    // Marker must be recorded.
    let marker_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM archive_migrations WHERE name = 'add_cache_read_tokens'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(marker_count, 1, "M2: migration marker must be recorded");
}

/// Re-opening after M2 has run is a no-op — marker present, columns present, no error.
#[test]
fn migration_m2_reopen_twice_is_idempotent() {
    use tempfile::NamedTempFile;
    let db_file = NamedTempFile::new().unwrap();
    let db_path = db_file.path().to_path_buf();

    // First open runs both M1 and M2.
    open_archive_db(&db_path).expect("first open must succeed");
    // Second open must not error even though both columns already exist.
    let conn2 = open_archive_db(&db_path).expect("second open must succeed (M2 is idempotent)");

    let marker_count: i64 = conn2
        .query_row(
            "SELECT COUNT(*) FROM archive_migrations WHERE name = 'add_cache_read_tokens'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        marker_count, 1,
        "M2: marker must still be present after idempotent second open"
    );
}

/// Partial-schema repair: only `turn_cache_read_tokens` present (no cumulative).
/// M2 must add the missing cumulative column and commit the marker.
#[test]
fn migration_m2_repairs_turn_only_partial_schema_and_records_marker() {
    use tempfile::NamedTempFile;
    let db_file = NamedTempFile::new().unwrap();
    let db_path = db_file.path().to_path_buf();

    // Build a DB that has `turn_cache_read_tokens` but NOT `cumulative_cache_read_tokens`
    // (simulates a crash between the two ALTER TABLE statements in old M2).
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.pragma_update(None, "busy_timeout", 5000).unwrap();
        // Use M1-era schema + add only the turn column manually.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS archived_events (
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
                harness TEXT,
                parse_status TEXT NOT NULL CHECK (parse_status IN ('valid','invalid')),
                turn_cache_read_tokens TEXT,
                PRIMARY KEY (identity_pubkey, relay_url, id)
            );
            CREATE TABLE IF NOT EXISTS archive_migrations (
                name TEXT PRIMARY KEY,
                applied_at INTEGER NOT NULL
            );",
        )
        .unwrap();
        // Mark M1 as done so open_archive_db skips it.
        conn.execute(
            "INSERT INTO archive_migrations (name, applied_at) VALUES ('add_harness_to_metric_index', 1)",
            [],
        )
        .unwrap();
        // Do NOT mark M2 — let open_archive_db run it.
    }

    // Opening must run M2: it finds turn_cache_read_tokens already present and
    // only adds the missing cumulative column, then commits the marker.
    let conn =
        open_archive_db(&db_path).expect("open_archive_db must repair turn-only partial M2 schema");

    let columns: Vec<String> = {
        let mut stmt = conn
            .prepare("PRAGMA table_info(agent_metric_index)")
            .unwrap();
        stmt.query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    assert!(
        columns.iter().any(|c| c == "turn_cache_read_tokens"),
        "M2 partial repair: turn_cache_read_tokens must be present"
    );
    assert!(
        columns.iter().any(|c| c == "cumulative_cache_read_tokens"),
        "M2 partial repair: cumulative_cache_read_tokens must have been added"
    );

    let marker_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM archive_migrations WHERE name = 'add_cache_read_tokens'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        marker_count, 1,
        "M2 partial repair: marker must be committed after full repair"
    );
}

/// Partial-schema repair: only `cumulative_cache_read_tokens` present (no turn).
/// M2 must add the missing turn column and commit the marker.
#[test]
fn migration_m2_repairs_cumulative_only_partial_schema_and_records_marker() {
    use tempfile::NamedTempFile;
    let db_file = NamedTempFile::new().unwrap();
    let db_path = db_file.path().to_path_buf();

    {
        let conn = Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.pragma_update(None, "busy_timeout", 5000).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS archived_events (
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
                harness TEXT,
                parse_status TEXT NOT NULL CHECK (parse_status IN ('valid','invalid')),
                cumulative_cache_read_tokens TEXT,
                PRIMARY KEY (identity_pubkey, relay_url, id)
            );
            CREATE TABLE IF NOT EXISTS archive_migrations (
                name TEXT PRIMARY KEY,
                applied_at INTEGER NOT NULL
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO archive_migrations (name, applied_at) VALUES ('add_harness_to_metric_index', 1)",
            [],
        )
        .unwrap();
    }

    let conn = open_archive_db(&db_path)
        .expect("open_archive_db must repair cumulative-only partial M2 schema");

    let columns: Vec<String> = {
        let mut stmt = conn
            .prepare("PRAGMA table_info(agent_metric_index)")
            .unwrap();
        stmt.query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    assert!(
        columns.iter().any(|c| c == "turn_cache_read_tokens"),
        "M2 partial repair: turn_cache_read_tokens must have been added"
    );
    assert!(
        columns.iter().any(|c| c == "cumulative_cache_read_tokens"),
        "M2 partial repair: cumulative_cache_read_tokens must be present"
    );

    let marker_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM archive_migrations WHERE name = 'add_cache_read_tokens'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        marker_count, 1,
        "M2 partial repair: marker must be committed after full repair"
    );
}

/// A fresh DB must include the M3 columns (`turn_cache_write_tokens`,
/// `cumulative_cache_write_tokens`, `pricing_authority`, `pricing_model`,
/// `pricing_cache_class`) in the initial schema.
#[test]
fn migration_m3_fresh_db_has_all_m3_columns() {
    use tempfile::NamedTempFile;
    let db_file = NamedTempFile::new().unwrap();
    let db_path = db_file.path().to_path_buf();
    let conn = open_archive_db(&db_path).expect("fresh open must succeed");

    let columns: Vec<String> = {
        let mut stmt = conn
            .prepare("PRAGMA table_info(agent_metric_index)")
            .unwrap();
        stmt.query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };

    for col in &[
        "turn_cache_write_tokens",
        "cumulative_cache_write_tokens",
        "pricing_authority",
        "pricing_model",
        "pricing_cache_class",
    ] {
        assert!(
            columns.iter().any(|c| c == col),
            "M3: {col} must exist on a fresh DB"
        );
    }

    let marker_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM archive_migrations WHERE name = 'add_cache_write_and_pricing'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(marker_count, 1, "M3: marker must be recorded on fresh DB");
}

/// Opening a post-M2, pre-M3 DB (has cache-read columns and M2 marker, but
/// no cache-write or pricing columns) must add all five M3 columns and
/// leave pre-existing rows as NULL in those columns.
#[test]
fn migration_m3_upgrades_post_m2_database() {
    use rusqlite::Connection;
    use tempfile::NamedTempFile;

    let db_file = NamedTempFile::new().unwrap();
    let db_path = db_file.path().to_path_buf();

    // Build a post-M2, pre-M3 DB by hand (bypassing the normal open path
    // so the M3 migration has not yet run).
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS archive_migrations (
                 name TEXT PRIMARY KEY,
                 applied_at INTEGER NOT NULL
             );
             CREATE TABLE agent_metric_index (
                 identity_pubkey TEXT NOT NULL,
                 relay_url       TEXT NOT NULL,
                 id              TEXT NOT NULL,
                 agent_pubkey    TEXT NOT NULL,
                 event_created_at INTEGER NOT NULL,
                 archived_at     INTEGER NOT NULL,
                 reported_at     INTEGER,
                 session_id      TEXT,
                 turn_seq        TEXT,
                 model           TEXT,
                 delta_reliable  INTEGER,
                 turn_input_tokens  TEXT,
                 turn_output_tokens TEXT,
                 turn_total_tokens  TEXT,
                 turn_cost_usd      REAL,
                 turn_cache_read_tokens       TEXT,
                 cumulative_input_tokens      TEXT,
                 cumulative_output_tokens     TEXT,
                 cumulative_total_tokens      TEXT,
                 cumulative_cost_usd          REAL,
                 cumulative_cache_read_tokens TEXT,
                 harness          TEXT,
                 parse_status     TEXT NOT NULL CHECK (parse_status IN ('valid','invalid')),
                 PRIMARY KEY (identity_pubkey, relay_url, id)
             );",
        )
        .unwrap();
        // Mark M1 and M2 as applied, but NOT M3.
        conn.execute_batch(
            "INSERT OR IGNORE INTO archive_migrations (name, applied_at) VALUES ('add_harness_to_metric_index', 0);
             INSERT OR IGNORE INTO archive_migrations (name, applied_at) VALUES ('add_cache_read_tokens', 0);",
        )
        .unwrap();
        // Insert a pre-migration row.
        conn.execute(
            "INSERT INTO agent_metric_index
                 (identity_pubkey, relay_url, id, agent_pubkey, event_created_at,
                  archived_at, reported_at, session_id, turn_seq, model,
                  delta_reliable, turn_input_tokens, turn_output_tokens,
                  turn_total_tokens, turn_cost_usd, turn_cache_read_tokens,
                  cumulative_input_tokens, cumulative_output_tokens,
                  cumulative_total_tokens, cumulative_cost_usd,
                  cumulative_cache_read_tokens, harness, parse_status)
             VALUES ('id','relay','eid-m3','agent1',100,200,1751414400,
                     's1','00000000000000000001','model1',
                     1,'00000000000000000010','00000000000000000020',
                     '00000000000000000030',0.01,'00000000000000000050',
                     '00000000000000000100','00000000000000000200',
                     '00000000000000000300',0.1,'00000000000000000500',
                     'buzz-agent','valid')",
            [],
        )
        .unwrap();
    }

    // Open via the production path — M3 should run.
    let conn = open_archive_db(&db_path).expect("open_archive_db must succeed on post-M2 DB");

    // All five M3 columns must now exist.
    let columns: Vec<String> = {
        let mut stmt = conn
            .prepare("PRAGMA table_info(agent_metric_index)")
            .unwrap();
        stmt.query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    for col in &[
        "turn_cache_write_tokens",
        "cumulative_cache_write_tokens",
        "pricing_authority",
        "pricing_model",
        "pricing_cache_class",
    ] {
        assert!(
            columns.iter().any(|c| c == col),
            "M3: {col} must be present after upgrade"
        );
    }

    // Pre-existing row gets NULLs for all new columns.
    type M3NullRow = (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let (tcw, ccw, pa, pm, pcc): M3NullRow = conn
        .query_row(
            "SELECT turn_cache_write_tokens, cumulative_cache_write_tokens, \
             pricing_authority, pricing_model, pricing_cache_class \
             FROM agent_metric_index WHERE id = 'eid-m3'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert!(
        tcw.is_none(),
        "M3: pre-existing row turn_cache_write_tokens must be NULL"
    );
    assert!(
        ccw.is_none(),
        "M3: pre-existing row cumulative_cache_write_tokens must be NULL"
    );
    assert!(
        pa.is_none(),
        "M3: pre-existing row pricing_authority must be NULL"
    );
    assert!(
        pm.is_none(),
        "M3: pre-existing row pricing_model must be NULL"
    );
    assert!(
        pcc.is_none(),
        "M3: pre-existing row pricing_cache_class must be NULL"
    );

    // Marker must be recorded.
    let marker_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM archive_migrations WHERE name = 'add_cache_write_and_pricing'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(marker_count, 1, "M3: migration marker must be recorded");
}

/// Re-opening after M3 has run is a no-op.
#[test]
fn migration_m3_reopen_twice_is_idempotent() {
    use tempfile::NamedTempFile;
    let db_file = NamedTempFile::new().unwrap();
    let db_path = db_file.path().to_path_buf();

    open_archive_db(&db_path).expect("first open must succeed");
    let conn2 = open_archive_db(&db_path).expect("second open must succeed (M3 is idempotent)");

    let marker_count: i64 = conn2
        .query_row(
            "SELECT COUNT(*) FROM archive_migrations WHERE name = 'add_cache_write_and_pricing'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        marker_count, 1,
        "M3: marker must still be present after idempotent second open"
    );
}
