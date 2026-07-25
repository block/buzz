//! Migration tests for `archive/store.rs` — M1: harness column.
//!
//! Kept in a sibling file so `store_tests.rs` stays under the 1000-line gate;
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
