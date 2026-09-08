//! Unit tests for `archive/metric_store.rs`.
//!
//! Kept in a sibling file so `metric_store.rs` stays under the file-size
//! gate; `#[path]`-included from there.

use super::*;
use crate::archive::store::{self, SCHEMA};

fn in_memory() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn.pragma_update(None, "busy_timeout", 5000).unwrap();
    conn.execute_batch(SCHEMA).unwrap();
    conn
}

fn valid_payload_json(session_id: &str, seq: u64, timestamp: &str) -> String {
    format!(
        r#"{{"harness":"goose","model":"claude","channelId":null,"sessionId":"{session_id}","turnId":null,"turnSeq":{seq},"timestamp":"{timestamp}","turn":{{"inputTokens":10,"outputTokens":20,"totalTokens":30,"costUsd":0.01}},"cumulative":{{"inputTokens":100,"outputTokens":200,"totalTokens":300,"costUsd":0.1}},"deltaReliable":true,"stopReason":"end_turn"}}"#
    )
}

#[allow(clippy::too_many_arguments)]
fn insert_archived_event(
    conn: &Connection,
    identity: &str,
    relay: &str,
    id: &str,
    kind: i64,
    pubkey: &str,
    created_at: i64,
    raw_json: &str,
    archived_at: i64,
) {
    store::upsert_archived_event(
        conn,
        identity,
        relay,
        id,
        kind,
        pubkey,
        created_at,
        raw_json,
        archived_at,
    )
    .unwrap();
}

// ── u64 sortable encoding ────────────────────────────────────────────────────

#[test]
fn u64_sortable_round_trips_zero_and_max() {
    for v in [0u64, 1, 12345, u64::MAX - 1, u64::MAX] {
        let encoded = encode_u64_sortable(v);
        assert_eq!(encoded.len(), U64_SORTABLE_WIDTH);
        assert_eq!(decode_u64_sortable(&encoded), Some(v));
    }
}

#[test]
fn u64_sortable_encoding_preserves_numeric_order_across_i64_max() {
    let below = i64::MAX as u64;
    let above = (i64::MAX as u64) + 1;
    let e_below = encode_u64_sortable(below);
    let e_above = encode_u64_sortable(above);
    assert!(
        e_below < e_above,
        "lexicographic order must match numeric order"
    );
}

#[test]
fn decode_rejects_wrong_width() {
    assert_eq!(decode_u64_sortable("123"), None);
    assert_eq!(decode_u64_sortable(""), None);
}

// ── from_payload parsing ─────────────────────────────────────────────────────

#[test]
fn from_payload_parses_valid_row() {
    let json = valid_payload_json("s1", 7, "2026-07-01T20:11:03.213Z");
    let row = AgentMetricIndexRow::from_payload(&json, "eid1", "agent1", 100, 200);
    assert_eq!(row.parse_status, ParseStatus::Valid);
    assert_eq!(row.session_id, Some("s1".to_string()));
    assert_eq!(row.turn_seq, Some(7));
    assert_eq!(row.turn_input_tokens, Some(10));
    assert_eq!(row.cumulative_input_tokens, Some(100));
    assert_eq!(row.model, Some("claude".to_string()));
    assert_eq!(row.harness, Some("goose".to_string()));
}

#[test]
fn from_payload_parses_harness_field() {
    let json = r#"{"harness":"claude-code","model":"claude-sonnet","timestamp":"2026-07-01T20:11:03Z","turn":{"inputTokens":5,"outputTokens":null,"totalTokens":null,"costUsd":null}}"#;
    let row = AgentMetricIndexRow::from_payload(json, "eid1", "agent1", 100, 200);
    assert_eq!(row.parse_status, ParseStatus::Valid);
    assert_eq!(row.harness, Some("claude-code".to_string()));
}

#[test]
fn from_payload_marks_unparseable_json_invalid() {
    let row = AgentMetricIndexRow::from_payload("not json", "eid1", "agent1", 100, 200);
    assert_eq!(row.parse_status, ParseStatus::Invalid);
    assert_eq!(row.turn_input_tokens, None);
}

#[test]
fn from_payload_marks_unparseable_timestamp_invalid() {
    let json = r#"{"harness":"goose","timestamp":"not-a-timestamp"}"#;
    let row = AgentMetricIndexRow::from_payload(json, "eid1", "agent1", 100, 200);
    assert_eq!(row.parse_status, ParseStatus::Invalid);
}

#[test]
fn from_payload_marks_cumulative_without_session_seq_invalid() {
    // cumulative present but sessionId/turnSeq missing — semantic-invalid per A5.
    let json = r#"{"harness":"goose","timestamp":"2026-07-01T20:11:03Z","cumulative":{"inputTokens":1,"outputTokens":null,"totalTokens":null,"costUsd":null}}"#;
    let row = AgentMetricIndexRow::from_payload(json, "eid1", "agent1", 100, 200);
    assert_eq!(row.parse_status, ParseStatus::Invalid);
}

#[test]
fn from_payload_accepts_missing_cumulative_without_session_seq() {
    // No cumulative object at all — session/seq are not required.
    let json = r#"{"harness":"goose","timestamp":"2026-07-01T20:11:03Z","turn":{"inputTokens":5,"outputTokens":null,"totalTokens":null,"costUsd":null}}"#;
    let row = AgentMetricIndexRow::from_payload(json, "eid1", "agent1", 100, 200);
    assert_eq!(row.parse_status, ParseStatus::Valid);
    assert_eq!(row.turn_input_tokens, Some(5));
}

// ── insert / idempotence ──────────────────────────────────────────────────────

#[test]
fn insert_metric_index_row_is_idempotent_on_pk() {
    let conn = in_memory();
    let row = AgentMetricIndexRow::from_payload(
        &valid_payload_json("s1", 1, "2026-07-01T00:00:00Z"),
        "eid1",
        "agent1",
        100,
        200,
    );
    let first = insert_metric_index_row(&conn, "id", "relay", &row).unwrap();
    let second = insert_metric_index_row(&conn, "id", "relay", &row).unwrap();
    assert!(first);
    assert!(!second, "second insert of the same PK must be a no-op");

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM agent_metric_index", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn insert_metric_index_row_round_trips_u64_max() {
    let conn = in_memory();
    let row = AgentMetricIndexRow {
        turn_seq: Some(u64::MAX),
        turn_input_tokens: Some(u64::MAX),
        cumulative_input_tokens: Some(u64::MAX),
        ..AgentMetricIndexRow::from_payload(
            &valid_payload_json("s1", 1, "2026-07-01T00:00:00Z"),
            "eid1",
            "agent1",
            100,
            200,
        )
    };
    insert_metric_index_row(&conn, "id", "relay", &row).unwrap();

    let loaded = load_window_valid_rows(&conn, "id", "relay", 0, i64::MAX, None).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].turn_seq, Some(u64::MAX));
    assert_eq!(loaded[0].turn_input_tokens, Some(u64::MAX));
    assert_eq!(loaded[0].cumulative_input_tokens, Some(u64::MAX));
}

#[test]
fn insert_invalid_row_preserves_null_parsed_columns() {
    let conn = in_memory();
    let row = AgentMetricIndexRow::from_payload("bad json", "eid1", "agent1", 100, 200);
    insert_metric_index_row(&conn, "id", "relay", &row).unwrap();

    let count = count_invalid_rows_in_window(&conn, "id", "relay", 0, i64::MAX, None).unwrap();
    assert_eq!(count, 1);
}

// ── Identity/relay isolation ─────────────────────────────────────────────────

#[test]
fn load_window_valid_rows_scoped_to_identity_and_relay() {
    let conn = in_memory();
    let row_a = AgentMetricIndexRow::from_payload(
        &valid_payload_json("s1", 1, "2026-07-01T00:00:00Z"),
        "eidA",
        "agent1",
        100,
        200,
    );
    let row_b = AgentMetricIndexRow::from_payload(
        &valid_payload_json("s1", 1, "2026-07-01T00:00:00Z"),
        "eidB",
        "agent1",
        100,
        200,
    );
    insert_metric_index_row(&conn, "identityA", "relay1", &row_a).unwrap();
    insert_metric_index_row(&conn, "identityB", "relay1", &row_b).unwrap();

    let loaded_a = load_window_valid_rows(&conn, "identityA", "relay1", 0, i64::MAX, None).unwrap();
    assert_eq!(loaded_a.len(), 1);
    assert_eq!(loaded_a[0].id, "eidA");

    let loaded_b = load_window_valid_rows(&conn, "identityB", "relay1", 0, i64::MAX, None).unwrap();
    assert_eq!(loaded_b.len(), 1);
    assert_eq!(loaded_b[0].id, "eidB");
}

#[test]
fn load_window_valid_rows_filters_by_agent_pubkey() {
    let conn = in_memory();
    let row_a = AgentMetricIndexRow::from_payload(
        &valid_payload_json("s1", 1, "2026-07-01T00:00:00Z"),
        "eidA",
        "agentA",
        100,
        200,
    );
    let row_b = AgentMetricIndexRow::from_payload(
        &valid_payload_json("s1", 1, "2026-07-01T00:00:00Z"),
        "eidB",
        "agentB",
        100,
        200,
    );
    insert_metric_index_row(&conn, "id", "relay", &row_a).unwrap();
    insert_metric_index_row(&conn, "id", "relay", &row_b).unwrap();

    let loaded = load_window_valid_rows(&conn, "id", "relay", 0, i64::MAX, Some("agentA")).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].agent_pubkey, "agentA");
}

#[test]
fn load_window_valid_rows_excludes_out_of_window_reported_at() {
    let conn = in_memory();
    let in_window = AgentMetricIndexRow::from_payload(
        &valid_payload_json("s1", 1, "2026-01-02T00:00:00Z"),
        "eid_in",
        "agent1",
        0,
        0,
    );
    let out_of_window = AgentMetricIndexRow::from_payload(
        &valid_payload_json("s1", 2, "2020-01-01T00:00:00Z"),
        "eid_out",
        "agent1",
        0,
        0,
    );
    insert_metric_index_row(&conn, "id", "relay", &in_window).unwrap();
    insert_metric_index_row(&conn, "id", "relay", &out_of_window).unwrap();

    let start = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
        .unwrap()
        .timestamp();
    let end = chrono::DateTime::parse_from_rfc3339("2026-01-03T00:00:00Z")
        .unwrap()
        .timestamp();
    let loaded = load_window_valid_rows(&conn, "id", "relay", start, end, None).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, "eid_in");
}

// ── load_rows_at_exact_keys ───────────────────────────────────────────────────

#[test]
fn load_rows_at_exact_keys_matches_multiple_groups() {
    let conn = in_memory();
    let r1 = AgentMetricIndexRow::from_payload(
        &valid_payload_json("s1", 5, "2026-07-01T00:00:00Z"),
        "e1",
        "agent1",
        0,
        0,
    );
    let r2 = AgentMetricIndexRow::from_payload(
        &valid_payload_json("s2", 9, "2026-07-01T00:00:00Z"),
        "e2",
        "agent1",
        0,
        0,
    );
    let r3_not_requested = AgentMetricIndexRow::from_payload(
        &valid_payload_json("s1", 99, "2026-07-01T00:00:00Z"),
        "e3",
        "agent1",
        0,
        0,
    );
    insert_metric_index_row(&conn, "id", "relay", &r1).unwrap();
    insert_metric_index_row(&conn, "id", "relay", &r2).unwrap();
    insert_metric_index_row(&conn, "id", "relay", &r3_not_requested).unwrap();

    let mut keys = std::collections::HashSet::new();
    keys.insert(("agent1".to_string(), "s1".to_string(), 5u64));
    keys.insert(("agent1".to_string(), "s2".to_string(), 9u64));

    let loaded = load_rows_at_exact_keys(&conn, "id", "relay", &keys).unwrap();
    let mut ids: Vec<&str> = loaded.iter().map(|r| r.id.as_str()).collect();
    ids.sort();
    assert_eq!(ids, vec!["e1", "e2"]);
}

#[test]
fn load_rows_at_exact_keys_returns_all_duplicates_at_one_key() {
    let conn = in_memory();
    let dup_a = AgentMetricIndexRow::from_payload(
        &valid_payload_json("s1", 5, "2026-07-01T00:00:00Z"),
        "eA",
        "agent1",
        0,
        0,
    );
    let dup_b = AgentMetricIndexRow::from_payload(
        &valid_payload_json("s1", 5, "2026-07-01T00:00:01Z"),
        "eB",
        "agent1",
        0,
        1,
    );
    insert_metric_index_row(&conn, "id", "relay", &dup_a).unwrap();
    insert_metric_index_row(&conn, "id", "relay", &dup_b).unwrap();

    let mut keys = std::collections::HashSet::new();
    keys.insert(("agent1".to_string(), "s1".to_string(), 5u64));
    let loaded = load_rows_at_exact_keys(&conn, "id", "relay", &keys).unwrap();
    assert_eq!(loaded.len(), 2);
}

// ── has_archived_evidence ─────────────────────────────────────────────────────

#[test]
fn has_archived_evidence_true_for_either_parse_status() {
    let conn = in_memory();
    let invalid_row = AgentMetricIndexRow::from_payload("bad", "eid1", "agentX", 0, 0);
    insert_metric_index_row(&conn, "id", "relay", &invalid_row).unwrap();

    assert!(has_archived_evidence(&conn, "id", "relay", "agentX").unwrap());
    assert!(!has_archived_evidence(&conn, "id", "relay", "agentY").unwrap());
}

#[test]
fn has_archived_evidence_ignores_bucket_boundaries() {
    let conn = in_memory();
    // A very old row (outside any realistic window) still counts as evidence.
    let old_row = AgentMetricIndexRow::from_payload(
        &valid_payload_json("s1", 1, "1999-01-01T00:00:00Z"),
        "eid1",
        "agentX",
        0,
        0,
    );
    insert_metric_index_row(&conn, "id", "relay", &old_row).unwrap();
    assert!(has_archived_evidence(&conn, "id", "relay", "agentX").unwrap());
}

// ── Backfill ──────────────────────────────────────────────────────────────────

#[test]
fn backfill_indexes_existing_unindexed_kind_44200_rows() {
    let conn = in_memory();
    let json = valid_payload_json("s1", 1, "2026-07-01T00:00:00Z");
    insert_archived_event(
        &conn, "id", "relay", "eid1", 44200, "agent1", 100, &json, 200,
    );

    let indexed = backfill_agent_metric_index(&conn, "id", "relay").unwrap();
    assert_eq!(indexed, 1);

    let loaded = load_window_valid_rows(&conn, "id", "relay", 0, i64::MAX, None).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, "eid1");
}

#[test]
fn backfill_is_idempotent_anti_join() {
    let conn = in_memory();
    let json = valid_payload_json("s1", 1, "2026-07-01T00:00:00Z");
    insert_archived_event(
        &conn, "id", "relay", "eid1", 44200, "agent1", 100, &json, 200,
    );

    backfill_agent_metric_index(&conn, "id", "relay").unwrap();
    let second_run = backfill_agent_metric_index(&conn, "id", "relay").unwrap();
    assert_eq!(second_run, 0, "second backfill run must index nothing new");

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM agent_metric_index", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn backfill_processes_chunks_larger_than_500_rows() {
    let conn = in_memory();
    // 501 rows exercises the CHUNK_SIZE=500 boundary.
    for i in 0..501 {
        let json = valid_payload_json(&format!("s{i}"), 1, "2026-07-01T00:00:00Z");
        insert_archived_event(
            &conn,
            "id",
            "relay",
            &format!("eid{i}"),
            44200,
            "agent1",
            100 + i as i64,
            &json,
            200,
        );
    }
    let indexed = backfill_agent_metric_index(&conn, "id", "relay").unwrap();
    assert_eq!(indexed, 501);

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM agent_metric_index", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 501);
}

#[test]
fn backfill_ignores_non_44200_rows() {
    let conn = in_memory();
    insert_archived_event(&conn, "id", "relay", "eid1", 1, "author1", 100, "{}", 200);
    let indexed = backfill_agent_metric_index(&conn, "id", "relay").unwrap();
    assert_eq!(indexed, 0);
}

// ── GC / orphan repair ────────────────────────────────────────────────────────

#[test]
fn delete_orphaned_metric_index_rows_removes_rows_with_no_canonical_event() {
    let conn = in_memory();
    // Planted orphan: index row with no matching archived_events row.
    let orphan = AgentMetricIndexRow::from_payload(
        &valid_payload_json("s1", 1, "2026-07-01T00:00:00Z"),
        "orphan_id",
        "agent1",
        0,
        0,
    );
    insert_metric_index_row(&conn, "id", "relay", &orphan).unwrap();

    let deleted = delete_orphaned_metric_index_rows(&conn, "id", "relay").unwrap();
    assert_eq!(deleted, 1);

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM agent_metric_index", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn delete_orphaned_metric_index_rows_preserves_rows_with_canonical_event() {
    let conn = in_memory();
    let json = valid_payload_json("s1", 1, "2026-07-01T00:00:00Z");
    insert_archived_event(
        &conn, "id", "relay", "eid1", 44200, "agent1", 100, &json, 200,
    );
    let row = AgentMetricIndexRow::from_payload(&json, "eid1", "agent1", 100, 200);
    insert_metric_index_row(&conn, "id", "relay", &row).unwrap();

    let deleted = delete_orphaned_metric_index_rows(&conn, "id", "relay").unwrap();
    assert_eq!(deleted, 0);
}

#[test]
fn repair_orphaned_metric_index_rows_self_heals_planted_orphan_before_read() {
    let conn = in_memory();
    let orphan = AgentMetricIndexRow::from_payload(
        &valid_payload_json("s1", 1, "2026-07-01T00:00:00Z"),
        "orphan_id",
        "agent1",
        0,
        0,
    );
    insert_metric_index_row(&conn, "id", "relay", &orphan).unwrap();

    repair_orphaned_metric_index_rows(&conn, "id", "relay").unwrap();
    let loaded = load_window_valid_rows(&conn, "id", "relay", 0, i64::MAX, None).unwrap();
    assert!(
        loaded.is_empty(),
        "planted orphan must never be reported after repair"
    );
}

#[test]
fn gc_orphaned_events_cascades_to_metric_index_atomically() {
    let conn = in_memory();
    let json = valid_payload_json("s1", 1, "2026-07-01T00:00:00Z");
    insert_archived_event(
        &conn, "id", "relay", "eid1", 44200, "agent1", 100, &json, 200,
    );
    let row = AgentMetricIndexRow::from_payload(&json, "eid1", "agent1", 100, 200);
    insert_metric_index_row(&conn, "id", "relay", &row).unwrap();

    // Remove the last scope row so the event becomes orphaned, then GC.
    // (No scope row was ever added in this test, so the event is already
    // orphaned by construction — gc_orphaned_events should delete both the
    // canonical row and its index row in one transaction.)
    store::gc_orphaned_events(&conn, "id", "relay").unwrap();

    let event_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM archived_events", [], |r| r.get(0))
        .unwrap();
    let index_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM agent_metric_index", [], |r| r.get(0))
        .unwrap();
    assert_eq!(event_count, 0);
    assert_eq!(
        index_count, 0,
        "index row must not outlive its canonical event"
    );
}

// ── EXPLAIN QUERY PLAN index assertions (A7) ─────────────────────────────────

fn query_plan(conn: &Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) -> String {
    let explain_sql = format!("EXPLAIN QUERY PLAN {sql}");
    let mut stmt = conn.prepare(&explain_sql).unwrap();
    let mut rows = stmt.query(params).unwrap();
    let mut plan = String::new();
    while let Some(row) = rows.next().unwrap() {
        let detail: String = row.get(3).unwrap();
        plan.push_str(&detail);
        plan.push('\n');
    }
    plan
}

#[test]
fn backfill_anti_join_uses_partial_index() {
    let conn = in_memory();
    let plan = query_plan(
        &conn,
        "SELECT ae.id FROM archived_events ae
         WHERE ae.identity_pubkey = ?1 AND ae.relay_url = ?2 AND ae.kind = 44200
           AND ae.id NOT IN (SELECT id FROM agent_metric_index WHERE identity_pubkey = ?1 AND relay_url = ?2)",
        &[&"id", &"relay"],
    );
    assert!(
        plan.contains("idx_archived_events_agent_metric"),
        "backfill anti-join must use the partial index, plan was:\n{plan}"
    );
}

#[test]
fn window_scan_uses_reported_index() {
    let conn = in_memory();
    let plan = query_plan(
        &conn,
        "SELECT * FROM agent_metric_index
         WHERE identity_pubkey = ?1 AND relay_url = ?2 AND parse_status = 'valid'
           AND reported_at >= ?3 AND reported_at < ?4",
        &[&"id", &"relay", &0i64, &100i64],
    );
    assert!(
        plan.contains("idx_agent_metric_reported"),
        "window scan must use the reported-time index, plan was:\n{plan}"
    );
}

#[test]
fn predecessor_lookup_uses_session_index() {
    let conn = in_memory();
    let plan = query_plan(
        &conn,
        "SELECT * FROM agent_metric_index
         WHERE identity_pubkey = ?1 AND relay_url = ?2 AND parse_status = 'valid'
           AND agent_pubkey = ?3 AND session_id = ?4 AND turn_seq IN (?5)",
        &[&"id", &"relay", &"agent1", &"s1", &"00000000000000000005"],
    );
    assert!(
        plan.contains("idx_agent_metric_session"),
        "predecessor lookup must use the session index, plan was:\n{plan}"
    );
}

// ── Migration: old-shape rows get harness populated on rebuild ────────────────

/// Simulate a pre-harness archive row by inserting an archived_event and an
/// index row with harness = NULL (as the old schema would have stored it),
/// then running the backfill rebuild path to verify harness is populated.
#[test]
fn migration_old_shape_rows_get_harness_populated_after_rebuild() {
    let conn = in_memory();
    let json = valid_payload_json("s1", 1, "2026-07-01T00:00:00Z");
    // Seed an archived event.
    insert_archived_event(
        &conn, "id", "relay", "eid1", 44200, "agent1", 100, &json, 200,
    );
    // Seed an index row with harness explicitly NULL (simulates pre-migration row).
    conn.execute(
        "INSERT INTO agent_metric_index
             (identity_pubkey, relay_url, id, agent_pubkey, event_created_at,
              archived_at, reported_at, session_id, turn_seq, harness, model,
              delta_reliable, turn_input_tokens, turn_output_tokens,
              turn_total_tokens, turn_cost_usd, cumulative_input_tokens,
              cumulative_output_tokens, cumulative_total_tokens,
              cumulative_cost_usd, parse_status)
         VALUES ('id','relay','eid1','agent1',100,200,1751414400000,
                 's1','00000000000000000001',NULL,'claude',
                 1,'00000000000000000010','00000000000000000020',
                 '00000000000000000030',0.01,
                 '00000000000000000100','00000000000000000200',
                 '00000000000000000300',0.1,'valid')",
        [],
    )
    .unwrap();

    // Verify the row is there with NULL harness.
    let harness_before: Option<String> = conn
        .query_row(
            "SELECT harness FROM agent_metric_index WHERE id = 'eid1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        harness_before, None,
        "old-shape row should start with NULL harness"
    );

    // Simulate the migration: delete index rows and re-run backfill.
    conn.execute_batch("DELETE FROM agent_metric_index")
        .unwrap();
    backfill_agent_metric_index(&conn, "id", "relay").unwrap();

    // The rebuilt row must have harness populated.
    let harness_after: Option<String> = conn
        .query_row(
            "SELECT harness FROM agent_metric_index WHERE id = 'eid1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        harness_after,
        Some("goose".to_string()),
        "rebuilt row must have harness populated from raw_json"
    );
}

// ── cache_read_tokens parse and persist ───────────────────────────────────────

/// A payload carrying `cacheReadTokens` in both `turn` and `cumulative` must
/// round-trip through `from_payload` → `insert_metric_index_row` → `row_from_sql`.
#[test]
fn from_payload_parses_cache_read_tokens_when_present() {
    let json = r#"{"harness":"goose","model":"claude","channelId":null,"sessionId":"s1","turnId":null,"turnSeq":1,"timestamp":"2026-07-01T00:00:00Z","turn":{"inputTokens":100,"outputTokens":20,"totalTokens":120,"costUsd":0.01,"cacheReadTokens":80},"cumulative":{"inputTokens":500,"outputTokens":100,"totalTokens":600,"costUsd":0.05,"cacheReadTokens":400},"deltaReliable":true,"stopReason":"end_turn"}"#;
    let row = AgentMetricIndexRow::from_payload(json, "eid1", "agent1", 100, 200);
    assert_eq!(
        row.turn_cache_read_tokens,
        Some(80),
        "turn_cache_read_tokens must be parsed from turn.cacheReadTokens"
    );
    assert_eq!(
        row.cumulative_cache_read_tokens,
        Some(400),
        "cumulative_cache_read_tokens must be parsed from cumulative.cacheReadTokens"
    );
}

/// A payload WITHOUT `cacheReadTokens` must produce `None` in both fields.
#[test]
fn from_payload_cache_read_tokens_none_when_absent() {
    let json = valid_payload_json("s1", 1, "2026-07-01T00:00:00Z");
    let row = AgentMetricIndexRow::from_payload(&json, "eid1", "agent1", 100, 200);
    assert_eq!(
        row.turn_cache_read_tokens, None,
        "turn_cache_read_tokens must be None when cacheReadTokens absent"
    );
    assert_eq!(
        row.cumulative_cache_read_tokens, None,
        "cumulative_cache_read_tokens must be None when cacheReadTokens absent"
    );
}

/// `insert_metric_index_row` persists nonzero cache columns; `row_from_sql`
/// (via `load_window_valid_rows`) reads them back correctly.
#[test]
fn insert_and_load_round_trips_cache_read_tokens() {
    let conn = in_memory();
    let json = r#"{"harness":"goose","model":"claude","channelId":null,"sessionId":"s1","turnId":null,"turnSeq":1,"timestamp":"2026-07-01T00:00:00Z","turn":{"inputTokens":100,"outputTokens":20,"totalTokens":120,"costUsd":0.01,"cacheReadTokens":80},"cumulative":{"inputTokens":500,"outputTokens":100,"totalTokens":600,"costUsd":0.05,"cacheReadTokens":400},"deltaReliable":true,"stopReason":"end_turn"}"#;
    insert_archived_event(
        &conn, "id", "relay", "eid1", 44200, "agent1", 100, json, 200,
    );
    let row = AgentMetricIndexRow::from_payload(json, "eid1", "agent1", 100, 200);
    insert_metric_index_row(&conn, "id", "relay", &row).unwrap();

    // Use load_window_valid_rows to exercise the full read path.
    let rows = load_window_valid_rows(&conn, "id", "relay", 0, i64::MAX, None).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].turn_cache_read_tokens,
        Some(80),
        "turn_cache_read_tokens must survive insert + load round trip"
    );
    assert_eq!(
        rows[0].cumulative_cache_read_tokens,
        Some(400),
        "cumulative_cache_read_tokens must survive insert + load round trip"
    );
}

/// A row inserted without cache tokens loads back with `None` for both fields —
/// columns default to NULL and the decoder handles NULL correctly.
#[test]
fn insert_and_load_cache_read_tokens_null_when_absent() {
    let conn = in_memory();
    let json = &valid_payload_json("s1", 1, "2026-07-01T00:00:00Z");
    insert_archived_event(
        &conn, "id", "relay", "eid1", 44200, "agent1", 100, json, 200,
    );
    let row = AgentMetricIndexRow::from_payload(json, "eid1", "agent1", 100, 200);
    insert_metric_index_row(&conn, "id", "relay", &row).unwrap();

    let rows = load_window_valid_rows(&conn, "id", "relay", 0, i64::MAX, None).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].turn_cache_read_tokens, None,
        "turn_cache_read_tokens must be None when not present in payload"
    );
    assert_eq!(
        rows[0].cumulative_cache_read_tokens, None,
        "cumulative_cache_read_tokens must be None when not present in payload"
    );
}

// ── M3 cache-write and pricing columns ────────────────────────────────────────

/// M3 positive full-row round trip: a payload carrying nonzero cache-write
/// tokens and a pricingIdentity must survive `from_payload` → `insert` → `load`
/// with all five M3 columns intact.
#[test]
fn m3_insert_and_load_round_trips_cache_write_and_pricing() {
    let conn = in_memory();
    let json = r#"{"harness":"buzz-agent","model":"claude-opus-4-5","channelId":null,"sessionId":"s1","turnId":null,"turnSeq":1,"timestamp":"2026-07-01T00:00:00Z","turn":{"inputTokens":200,"outputTokens":50,"totalTokens":250,"costUsd":0.02,"cacheReadTokens":120,"cacheWriteTokens":30},"cumulative":{"inputTokens":600,"outputTokens":150,"totalTokens":750,"costUsd":0.06,"cacheReadTokens":360,"cacheWriteTokens":90},"deltaReliable":true,"stopReason":"end_turn","pricingIdentity":{"authority":"api.anthropic.com","model":"claude-opus-4-5"}}"#;
    insert_archived_event(
        &conn,
        "id",
        "relay",
        "eid-m3-pos",
        44200,
        "agent1",
        100,
        json,
        200,
    );
    let row = AgentMetricIndexRow::from_payload(json, "eid-m3-pos", "agent1", 100, 200);

    // Verify from_payload extracted the M3 fields.
    assert_eq!(
        row.turn_cache_write_tokens,
        Some(30),
        "from_payload: turn_cache_write_tokens must parse cacheWriteTokens"
    );
    assert_eq!(
        row.cumulative_cache_write_tokens,
        Some(90),
        "from_payload: cumulative_cache_write_tokens must parse cumulative.cacheWriteTokens"
    );
    assert_eq!(
        row.pricing_authority.as_deref(),
        Some("api.anthropic.com"),
        "from_payload: pricing_authority must parse pricingIdentity.authority"
    );
    assert_eq!(
        row.pricing_model.as_deref(),
        Some("claude-opus-4-5"),
        "from_payload: pricing_model must parse pricingIdentity.model"
    );
    assert!(
        row.pricing_cache_class.is_none(),
        "from_payload: pricing_cache_class must be None when absent"
    );

    insert_metric_index_row(&conn, "id", "relay", &row).unwrap();

    let rows = load_window_valid_rows(&conn, "id", "relay", 0, i64::MAX, None).unwrap();
    assert_eq!(rows.len(), 1);
    let loaded = &rows[0];
    assert_eq!(
        loaded.turn_cache_write_tokens,
        Some(30),
        "round-trip: turn_cache_write_tokens must survive insert + load"
    );
    assert_eq!(
        loaded.cumulative_cache_write_tokens,
        Some(90),
        "round-trip: cumulative_cache_write_tokens must survive insert + load"
    );
    assert_eq!(
        loaded.pricing_authority.as_deref(),
        Some("api.anthropic.com"),
        "round-trip: pricing_authority must survive insert + load"
    );
    assert_eq!(
        loaded.pricing_model.as_deref(),
        Some("claude-opus-4-5"),
        "round-trip: pricing_model must survive insert + load"
    );
    assert!(
        loaded.pricing_cache_class.is_none(),
        "round-trip: pricing_cache_class must be None (absent in payload)"
    );
}

/// M3 omission/explicit-zero: a payload with no cache-write and no pricingIdentity
/// must produce NULL for all five M3 columns — never inferred zeros.
#[test]
fn m3_insert_and_load_m3_columns_null_when_absent() {
    let conn = in_memory();
    // Standard payload without cacheWriteTokens or pricingIdentity.
    let json = &valid_payload_json("s1", 1, "2026-07-01T00:00:00Z");
    insert_archived_event(
        &conn,
        "id",
        "relay",
        "eid-m3-null",
        44200,
        "agent1",
        100,
        json,
        200,
    );
    let row = AgentMetricIndexRow::from_payload(json, "eid-m3-null", "agent1", 100, 200);
    insert_metric_index_row(&conn, "id", "relay", &row).unwrap();

    let rows = load_window_valid_rows(&conn, "id", "relay", 0, i64::MAX, None).unwrap();
    assert_eq!(rows.len(), 1);
    let loaded = &rows[0];
    assert_eq!(
        loaded.turn_cache_write_tokens, None,
        "M3: turn_cache_write_tokens must be None when absent in payload"
    );
    assert_eq!(
        loaded.cumulative_cache_write_tokens, None,
        "M3: cumulative_cache_write_tokens must be None when absent in payload"
    );
    assert_eq!(
        loaded.pricing_authority, None,
        "M3: pricing_authority must be None when pricingIdentity absent"
    );
    assert_eq!(
        loaded.pricing_model, None,
        "M3: pricing_model must be None when pricingIdentity absent"
    );
    assert_eq!(
        loaded.pricing_cache_class, None,
        "M3: pricing_cache_class must be None when pricingIdentity absent"
    );
}
