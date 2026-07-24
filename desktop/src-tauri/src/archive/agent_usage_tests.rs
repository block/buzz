//! Tests for the pure NIP-AM accounting ladder, accumulators, and request
//! validation. No SQLite — rows are constructed directly.

use super::*;
use crate::archive::metric_store::ParseStatus;

// ── Row builder ──────────────────────────────────────────────────────────────

/// Build a fully-specified valid row for test scenarios. Defaults every
/// optional field to `None`/appropriate zero; callers override what a
/// scenario needs via struct-update syntax.
fn row(id: &str, agent: &str, session: &str, seq: u64, reported_at: i64) -> AgentMetricIndexRow {
    AgentMetricIndexRow {
        id: id.to_string(),
        agent_pubkey: agent.to_string(),
        event_created_at: reported_at,
        archived_at: reported_at,
        reported_at: Some(reported_at),
        session_id: Some(session.to_string()),
        turn_seq: Some(seq),
        model: None,
        delta_reliable: Some(true),
        turn_input_tokens: None,
        turn_output_tokens: None,
        turn_total_tokens: None,
        turn_cost_usd: None,
        cumulative_input_tokens: None,
        cumulative_output_tokens: None,
        cumulative_total_tokens: None,
        cumulative_cost_usd: None,
        parse_status: ParseStatus::Valid,
    }
}

/// Standard 8-boundary window covering one day, seconds since epoch.
const DAY: i64 = 86_400;
fn boundaries_7() -> Vec<i64> {
    (0..=7).map(|i| i * DAY).collect()
}

fn probe_map(
    rows: &[AgentMetricIndexRow],
) -> std::collections::HashMap<(String, String, u64), Vec<&AgentMetricIndexRow>> {
    let mut m = std::collections::HashMap::new();
    for r in rows {
        if let Some(key) = r.accounting_key() {
            m.entry(key).or_insert_with(Vec::new).push(r);
        }
    }
    m
}

// ── Ladder: direct / cumulative / fallback ──────────────────────────────────

#[test]
fn direct_turn_value_used_when_no_baseline() {
    let r = AgentMetricIndexRow {
        turn_input_tokens: Some(100),
        delta_reliable: Some(true),
        ..row("e1", "agent1", "s1", 1, 0)
    };
    let __probe_rows = [r.clone()];
    let probes = probe_map(&__probe_rows);
    let outcome = compute_event_outcome(&r, &probes);
    assert!(matches!(outcome.input, FieldValue::Known(100)));
}

#[test]
fn adjacent_cumulative_preferred_over_direct() {
    let prev = AgentMetricIndexRow {
        cumulative_input_tokens: Some(1000),
        ..row("e0", "agent1", "s1", 1, 0)
    };
    let cur = AgentMetricIndexRow {
        cumulative_input_tokens: Some(1300),
        turn_input_tokens: Some(999), // deliberately wrong direct value
        delta_reliable: Some(true),
        ..row("e1", "agent1", "s1", 2, 10)
    };
    let __probe_rows = [prev, cur.clone()];
    let probes = probe_map(&__probe_rows);
    let outcome = compute_event_outcome(&cur, &probes);
    assert!(matches!(outcome.input, FieldValue::Known(300)));
}

#[test]
fn direct_fallback_when_baseline_missing() {
    // seq 5 has no row at seq 4 in the probe set → gap, direct-reliable only.
    let cur = AgentMetricIndexRow {
        cumulative_input_tokens: Some(500),
        turn_input_tokens: Some(42),
        delta_reliable: Some(true),
        ..row("e1", "agent1", "s1", 5, 0)
    };
    let __probe_rows = [cur.clone()];
    let probes = probe_map(&__probe_rows);
    let outcome = compute_event_outcome(&cur, &probes);
    assert!(matches!(outcome.input, FieldValue::Known(42)));
}

#[test]
fn unreliable_delta_with_no_baseline_is_unknown() {
    let cur = AgentMetricIndexRow {
        turn_input_tokens: Some(42),
        delta_reliable: Some(false),
        ..row("e1", "agent1", "s1", 5, 0)
    };
    let __probe_rows = [cur.clone()];
    let probes = probe_map(&__probe_rows);
    let outcome = compute_event_outcome(&cur, &probes);
    assert!(matches!(outcome.input, FieldValue::Unknown));
}

#[test]
fn sequence_gap_no_diff_but_direct_reliable_may_count() {
    // predecessor exists at seq 1 but current is seq 3 (gap at seq 2) —
    // predecessor lookup requires exact S-1, so seq 3's predecessor probe is
    // for seq 2, which is absent. Direct fallback applies.
    let baseline = AgentMetricIndexRow {
        cumulative_input_tokens: Some(100),
        ..row("e0", "agent1", "s1", 1, 0)
    };
    let cur = AgentMetricIndexRow {
        cumulative_input_tokens: Some(400),
        turn_input_tokens: Some(50),
        delta_reliable: Some(true),
        ..row("e1", "agent1", "s1", 3, 10)
    };
    let __probe_rows = [baseline, cur.clone()];
    let probes = probe_map(&__probe_rows);
    let outcome = compute_event_outcome(&cur, &probes);
    assert!(matches!(outcome.input, FieldValue::Known(50)));
}

// ── A1: counter decrease is terminal, no fallback ───────────────────────────

#[test]
fn adjacent_decrease_with_reliable_direct_present_is_unknown() {
    // Required A1 test: decreasing cumulative pair + deltaReliable true +
    // direct turn value present → field unknown, NOT the direct value.
    let prev = AgentMetricIndexRow {
        cumulative_input_tokens: Some(1000),
        ..row("e0", "agent1", "s1", 1, 0)
    };
    let cur = AgentMetricIndexRow {
        cumulative_input_tokens: Some(600), // decreased
        turn_input_tokens: Some(77),        // present, but must NOT be used
        delta_reliable: Some(true),
        ..row("e1", "agent1", "s1", 2, 10)
    };
    let __probe_rows = [prev, cur.clone()];
    let probes = probe_map(&__probe_rows);
    let outcome = compute_event_outcome(&cur, &probes);
    assert!(matches!(outcome.input, FieldValue::Unknown));
}

#[test]
fn decrease_taints_only_the_affected_field() {
    // Interpretation note in A1: a decrease on one field must not zero out
    // sibling fields whose own adjacent pair is nondecreasing.
    let prev = AgentMetricIndexRow {
        cumulative_input_tokens: Some(1000),
        cumulative_output_tokens: Some(200),
        ..row("e0", "agent1", "s1", 1, 0)
    };
    let cur = AgentMetricIndexRow {
        cumulative_input_tokens: Some(600),  // decreased
        cumulative_output_tokens: Some(250), // increased, still valid
        delta_reliable: Some(true),
        ..row("e1", "agent1", "s1", 2, 10)
    };
    let __probe_rows = [prev, cur.clone()];
    let probes = probe_map(&__probe_rows);
    let outcome = compute_event_outcome(&cur, &probes);
    assert!(matches!(outcome.input, FieldValue::Unknown));
    assert!(matches!(outcome.output, FieldValue::Known(50)));
}

// ── Cost ladder (mirrors the token ladder tests above, f64-specific) ───────

#[test]
fn ladder_cost_direct_value_used_when_no_baseline() {
    let r = AgentMetricIndexRow {
        turn_cost_usd: Some(0.05),
        delta_reliable: Some(true),
        ..row("e1", "agent1", "s1", 1, 0)
    };
    let __probe_rows = [r.clone()];
    let probes = probe_map(&__probe_rows);
    let outcome = compute_event_outcome(&r, &probes);
    assert!(matches!(outcome.cost, FieldValue::Known(v) if v == 0.05));
}

#[test]
fn ladder_cost_adjacent_cumulative_preferred_over_direct() {
    let prev = AgentMetricIndexRow {
        cumulative_cost_usd: Some(1.0),
        ..row("e0", "agent1", "s1", 1, 0)
    };
    let cur = AgentMetricIndexRow {
        cumulative_cost_usd: Some(1.3),
        turn_cost_usd: Some(999.0), // deliberately wrong direct value
        delta_reliable: Some(true),
        ..row("e1", "agent1", "s1", 2, 10)
    };
    let __probe_rows = [prev, cur.clone()];
    let probes = probe_map(&__probe_rows);
    let outcome = compute_event_outcome(&cur, &probes);
    assert!(matches!(outcome.cost, FieldValue::Known(v) if (v - 0.3).abs() < f64::EPSILON));
}

#[test]
fn ladder_cost_adjacent_decrease_is_unknown_not_direct() {
    // A1 applies identically to the cost field: a decreasing cumulative
    // pair is terminal-unknown even with a present direct value.
    let prev = AgentMetricIndexRow {
        cumulative_cost_usd: Some(5.0),
        ..row("e0", "agent1", "s1", 1, 0)
    };
    let cur = AgentMetricIndexRow {
        cumulative_cost_usd: Some(3.0), // decreased
        turn_cost_usd: Some(1.0),       // present, but must NOT be used
        delta_reliable: Some(true),
        ..row("e1", "agent1", "s1", 2, 10)
    };
    let __probe_rows = [prev, cur.clone()];
    let probes = probe_map(&__probe_rows);
    let outcome = compute_event_outcome(&cur, &probes);
    assert!(matches!(outcome.cost, FieldValue::Unknown));
}

// ── A4/A11: duplicate sequence quarantine ───────────────────────────────────

#[test]
fn duplicate_at_sequence_quarantines_successor() {
    // Two rows at N, one at N+1: N+1 must not cumulative-diff against
    // either N candidate.
    let dup_a = AgentMetricIndexRow {
        cumulative_input_tokens: Some(100),
        ..row("e0a", "agent1", "s1", 5, 0)
    };
    let dup_b = AgentMetricIndexRow {
        cumulative_input_tokens: Some(150),
        ..row("e0b", "agent1", "s1", 5, 1)
    };
    let successor = AgentMetricIndexRow {
        cumulative_input_tokens: Some(400),
        turn_input_tokens: Some(30),
        delta_reliable: Some(true),
        ..row("e1", "agent1", "s1", 6, 10)
    };
    let __probe_rows = [dup_a, dup_b, successor.clone()];
    let probes = probe_map(&__probe_rows);
    let outcome = compute_event_outcome(&successor, &probes);
    // No usable baseline (ambiguous predecessor) → direct-reliable fallback.
    assert!(matches!(outcome.input, FieldValue::Known(30)));
}

#[test]
fn duplicate_row_itself_gets_no_cumulative_delta() {
    let baseline = AgentMetricIndexRow {
        cumulative_input_tokens: Some(100),
        ..row("e_base", "agent1", "s1", 4, 0)
    };
    let dup_a = AgentMetricIndexRow {
        cumulative_input_tokens: Some(200),
        turn_input_tokens: Some(99),
        delta_reliable: Some(true),
        ..row("e5a", "agent1", "s1", 5, 1)
    };
    let dup_b = AgentMetricIndexRow {
        cumulative_input_tokens: Some(250),
        ..row("e5b", "agent1", "s1", 5, 2)
    };
    let __probe_rows = [baseline, dup_a.clone(), dup_b];
    let probes = probe_map(&__probe_rows);
    let outcome = compute_event_outcome(&dup_a, &probes);
    // Own sequence has >1 row → no cumulative delta; direct-reliable value
    // for dup_a specifically still counts (A4: "only an independently
    // reliable direct turn value may count").
    assert!(matches!(outcome.input, FieldValue::Known(99)));
}

#[test]
fn duplicate_out_of_window_peer_still_quarantines_in_window_row() {
    // A11: an out-of-window duplicate at the same sequence still poisons the
    // in-window row's cumulative eligibility, because the probe set has no
    // reported_at restriction.
    let out_of_window_dup = AgentMetricIndexRow {
        cumulative_input_tokens: Some(500),
        ..row("e_old", "agent1", "s1", 5, -1000)
    };
    let baseline = AgentMetricIndexRow {
        cumulative_input_tokens: Some(100),
        ..row("e_base", "agent1", "s1", 4, 0)
    };
    let in_window = AgentMetricIndexRow {
        cumulative_input_tokens: Some(200),
        turn_input_tokens: Some(11),
        delta_reliable: Some(true),
        ..row("e5", "agent1", "s1", 5, 1)
    };
    let __probe_rows = [out_of_window_dup, baseline, in_window.clone()];
    let probes = probe_map(&__probe_rows);
    let outcome = compute_event_outcome(&in_window, &probes);
    assert!(matches!(outcome.input, FieldValue::Known(11)));
}

// ── A12: checked_sub sequence arithmetic ────────────────────────────────────

#[test]
fn adjacent_pair_at_u64_max_computes_normally() {
    let prev = AgentMetricIndexRow {
        cumulative_input_tokens: Some(1000),
        ..row("e_prev", "agent1", "s1", u64::MAX - 1, 0)
    };
    let cur = AgentMetricIndexRow {
        cumulative_input_tokens: Some(1500),
        ..row("e_max", "agent1", "s1", u64::MAX, 10)
    };
    let __probe_rows = [prev, cur.clone()];
    let probes = probe_map(&__probe_rows);
    let outcome = compute_event_outcome(&cur, &probes);
    assert!(matches!(outcome.input, FieldValue::Known(500)));
}

#[test]
fn duplicate_at_u64_max_needs_no_successor_probe() {
    // u64::MAX has no successor sequence to quarantine — verify duplicate
    // handling at MAX itself still works (own-sequence cardinality check).
    let dup_a = AgentMetricIndexRow {
        cumulative_input_tokens: Some(100),
        turn_input_tokens: Some(5),
        delta_reliable: Some(true),
        ..row("e_max_a", "agent1", "s1", u64::MAX, 0)
    };
    let dup_b = AgentMetricIndexRow {
        cumulative_input_tokens: Some(150),
        ..row("e_max_b", "agent1", "s1", u64::MAX, 1)
    };
    let __probe_rows = [dup_a.clone(), dup_b];
    let probes = probe_map(&__probe_rows);
    let outcome = compute_event_outcome(&dup_a, &probes);
    assert!(matches!(outcome.input, FieldValue::Known(5)));
}

#[test]
fn seq_zero_has_no_baseline_underflow() {
    let cur = AgentMetricIndexRow {
        cumulative_input_tokens: Some(100),
        turn_input_tokens: Some(100),
        delta_reliable: Some(true),
        ..row("e0", "agent1", "s1", 0, 0)
    };
    let __probe_rows = [cur.clone()];
    let probes = probe_map(&__probe_rows);
    // Must not panic (checked_sub) and must fall back to direct.
    let outcome = compute_event_outcome(&cur, &probes);
    assert!(matches!(outcome.input, FieldValue::Known(100)));
}

// ── Null total / per-field independence ─────────────────────────────────────

#[test]
fn null_total_tokens_stays_unknown_even_when_input_output_known() {
    let r = AgentMetricIndexRow {
        turn_input_tokens: Some(10),
        turn_output_tokens: Some(20),
        turn_total_tokens: None,
        delta_reliable: Some(true),
        ..row("e1", "agent1", "s1", 1, 0)
    };
    let __probe_rows = [r.clone()];
    let probes = probe_map(&__probe_rows);
    let outcome = compute_event_outcome(&r, &probes);
    assert!(matches!(outcome.input, FieldValue::Known(10)));
    assert!(matches!(outcome.output, FieldValue::Known(20)));
    assert!(matches!(outcome.total, FieldValue::Unknown));
}

// ── Cross-session / cross-agent isolation ───────────────────────────────────

#[test]
fn cumulative_diff_never_crosses_session_boundary() {
    let other_session = AgentMetricIndexRow {
        cumulative_input_tokens: Some(999_999),
        ..row("e_other", "agent1", "s_other", 1, 0)
    };
    let cur = AgentMetricIndexRow {
        cumulative_input_tokens: Some(50),
        turn_input_tokens: Some(50),
        delta_reliable: Some(true),
        ..row("e1", "agent1", "s1", 1, 10)
    };
    let __probe_rows = [other_session, cur.clone()];
    let probes = probe_map(&__probe_rows);
    let outcome = compute_event_outcome(&cur, &probes);
    // seq 1 has no predecessor (seq 0) in ANY session — direct fallback.
    assert!(matches!(outcome.input, FieldValue::Known(50)));
}

#[test]
fn cumulative_diff_never_crosses_agent_boundary() {
    let other_agent = AgentMetricIndexRow {
        cumulative_input_tokens: Some(999_999),
        ..row("e_other", "agent2", "s1", 1, 0)
    };
    let cur = AgentMetricIndexRow {
        cumulative_input_tokens: Some(60),
        turn_input_tokens: Some(60),
        delta_reliable: Some(true),
        ..row("e1", "agent1", "s1", 2, 10)
    };
    let __probe_rows = [other_agent, cur.clone()];
    let probes = probe_map(&__probe_rows);
    let outcome = compute_event_outcome(&cur, &probes);
    // seq 2's predecessor (seq 1) does not exist for agent1 — direct.
    assert!(matches!(outcome.input, FieldValue::Known(60)));
}

// ── window_probe_keys ────────────────────────────────────────────────────────

#[test]
fn window_probe_keys_includes_own_and_predecessor() {
    let r = row("e1", "agent1", "s1", 5, 0);
    let keys = window_probe_keys(&[r]);
    assert!(keys.contains(&("agent1".to_string(), "s1".to_string(), 5)));
    assert!(keys.contains(&("agent1".to_string(), "s1".to_string(), 4)));
    assert_eq!(keys.len(), 2);
}

#[test]
fn window_probe_keys_skips_predecessor_at_seq_zero() {
    let r = row("e1", "agent1", "s1", 0, 0);
    let keys = window_probe_keys(&[r]);
    assert_eq!(keys.len(), 1);
    assert!(keys.contains(&("agent1".to_string(), "s1".to_string(), 0)));
}

#[test]
fn window_probe_keys_skips_rows_without_session_or_seq() {
    let r = AgentMetricIndexRow {
        session_id: None,
        turn_seq: None,
        ..row("e1", "agent1", "s1", 5, 0)
    };
    let keys = window_probe_keys(&[r]);
    assert!(keys.is_empty());
}

// ── assign_bucket_index: boundary edges ─────────────────────────────────────

#[test]
fn assign_bucket_index_start_is_inclusive_end_is_exclusive() {
    let boundaries = boundaries_7();
    // Exactly on bucket 1's start boundary → bucket 1, not bucket 0.
    assert_eq!(assign_bucket_index(&boundaries, DAY), Some(1));
    // One second before bucket 1's start → still bucket 0 (end-exclusive).
    assert_eq!(assign_bucket_index(&boundaries, DAY - 1), Some(0));
}

#[test]
fn assign_bucket_index_returns_none_outside_every_bucket() {
    let boundaries = boundaries_7();
    assert_eq!(assign_bucket_index(&boundaries, -1), None);
    assert_eq!(assign_bucket_index(&boundaries, boundaries[7]), None); // last boundary is exclusive end
}

// ── compute_series: bucketing, overflow, ranking, models ────────────────────

#[test]
fn compute_series_buckets_by_reported_at_not_created_at() {
    let r = AgentMetricIndexRow {
        turn_input_tokens: Some(10),
        delta_reliable: Some(true),
        event_created_at: 999_999_999, // deliberately wrong/misleading
        ..row("e1", "agent1", "s1", 1, DAY + 100)  // reported_at lands in bucket 1
    };
    let boundaries = boundaries_7();
    let series = compute_series(
        std::slice::from_ref(&r),
        std::slice::from_ref(&r),
        0,
        &boundaries,
        None,
        true,
    );
    assert_eq!(series.buckets[0].report_count, 0);
    assert_eq!(series.buckets[1].report_count, 1);
}

#[test]
fn compute_series_checked_add_overflow_marks_incomplete_without_wrapping() {
    let r1 = AgentMetricIndexRow {
        turn_input_tokens: Some(u64::MAX),
        delta_reliable: Some(true),
        ..row("e1", "agent1", "s1", 10, 0)
    };
    let r2 = AgentMetricIndexRow {
        turn_input_tokens: Some(5),
        delta_reliable: Some(true),
        ..row("e2", "agent1", "s2", 10, 1) // different session avoids adjacency
    };
    let boundaries = boundaries_7();
    let rows = vec![r1, r2];
    let series = compute_series(&rows, &rows, 0, &boundaries, None, true);
    let bucket = &series.buckets[0];
    assert!(bucket.has_unknown_usage);
    // Value freezes at the last valid sum (u64::MAX) rather than wrapping.
    assert_eq!(bucket.usage.input_tokens.value, Some(u64::MAX.to_string()));
    assert!(bucket.usage.input_tokens.incomplete);
}

#[test]
fn compute_series_cost_non_finite_marks_incomplete() {
    let r1 = AgentMetricIndexRow {
        turn_cost_usd: Some(f64::MAX),
        delta_reliable: Some(true),
        ..row("e1", "agent1", "s1", 10, 0)
    };
    let r2 = AgentMetricIndexRow {
        turn_cost_usd: Some(f64::MAX),
        delta_reliable: Some(true),
        ..row("e2", "agent1", "s2", 10, 1)
    };
    let boundaries = boundaries_7();
    let rows = vec![r1, r2];
    let series = compute_series(&rows, &rows, 0, &boundaries, None, true);
    assert!(series.buckets[0].usage.estimated_cost_usd.incomplete);
}

#[test]
fn compute_series_ranks_known_total_before_unknown_total() {
    let known = AgentMetricIndexRow {
        turn_total_tokens: Some(500),
        delta_reliable: Some(true),
        ..row("e1", "agent_known", "s1", 1, 0)
    };
    let unknown = AgentMetricIndexRow {
        turn_total_tokens: None,
        delta_reliable: Some(true),
        ..row("e2", "agent_unknown", "s1", 1, 0)
    };
    let boundaries = boundaries_7();
    let rows = vec![known, unknown];
    let series = compute_series(&rows, &rows, 0, &boundaries, None, true);
    assert_eq!(series.agents[0].agent_pubkey, "agent_known");
    assert_eq!(series.agents[1].agent_pubkey, "agent_unknown");
}

#[test]
fn compute_series_ties_on_known_total_break_by_pubkey_ascending() {
    // Equal totalTokens for two agents: the A2 tiebreak must be
    // deterministic pubkey order, not insertion/hash order.
    let agent_z = AgentMetricIndexRow {
        turn_total_tokens: Some(100),
        delta_reliable: Some(true),
        ..row("e1", "agent_zzz", "s1", 1, 0)
    };
    let agent_a = AgentMetricIndexRow {
        turn_total_tokens: Some(100),
        delta_reliable: Some(true),
        ..row("e2", "agent_aaa", "s1", 1, 0)
    };
    let boundaries = boundaries_7();
    let rows = vec![agent_z, agent_a];
    let series = compute_series(&rows, &rows, 0, &boundaries, None, true);
    assert_eq!(series.agents[0].agent_pubkey, "agent_aaa");
    assert_eq!(series.agents[1].agent_pubkey, "agent_zzz");
}

#[test]
fn compute_series_model_breakdown_attributes_per_event_model() {
    let r1 = AgentMetricIndexRow {
        model: Some("model-a".to_string()),
        turn_input_tokens: Some(10),
        delta_reliable: Some(true),
        ..row("e1", "agent1", "s1", 1, 0)
    };
    let r2 = AgentMetricIndexRow {
        model: Some("model-b".to_string()),
        turn_input_tokens: Some(20),
        delta_reliable: Some(true),
        ..row("e2", "agent1", "s2", 1, 1)
    };
    let boundaries = boundaries_7();
    let rows = vec![r1, r2];
    let series = compute_series(&rows, &rows, 0, &boundaries, None, true);
    assert_eq!(series.agents.len(), 1);
    assert_eq!(series.agents[0].models.len(), 2);
}

#[test]
fn compute_series_invalid_report_count_passed_through_and_not_bucketed() {
    let boundaries = boundaries_7();
    let series = compute_series(&[], &[], 3, &boundaries, None, true);
    assert_eq!(series.coverage.invalid_report_count, 3);
    assert_eq!(series.coverage.report_count, 0);
    assert!(series.coverage.has_unknown_usage);
}

#[test]
fn compute_series_zero_invalid_and_no_unknown_rows_is_not_unknown() {
    let boundaries = boundaries_7();
    let series = compute_series(&[], &[], 0, &boundaries, None, true);
    assert_eq!(series.coverage.invalid_report_count, 0);
    assert!(!series.coverage.has_unknown_usage);
}

#[test]
fn compute_series_collection_enabled_passthrough() {
    let boundaries = boundaries_7();
    let series = compute_series(&[], &[], 0, &boundaries, None, false);
    assert!(!series.collection_enabled);
}

#[test]
fn compute_series_has_archived_evidence_passthrough() {
    let boundaries = boundaries_7();
    let series = compute_series(&[], &[], 0, &boundaries, Some(true), true);
    assert_eq!(series.has_archived_evidence, Some(true));
    let series_none = compute_series(&[], &[], 0, &boundaries, None, true);
    assert_eq!(series_none.has_archived_evidence, None);
}

// ── validate_request ─────────────────────────────────────────────────────────

fn req(boundaries: Vec<i64>, agent_pubkey: Option<String>) -> AgentUsageSeriesRequest {
    AgentUsageSeriesRequest {
        bucket_boundaries: boundaries,
        agent_pubkey,
    }
}

#[test]
fn validate_request_accepts_8_and_31_boundaries() {
    assert!(validate_request(&req(boundaries_7(), None)).is_ok());
    let b31: Vec<i64> = (0..=30).map(|i| i * DAY).collect();
    assert!(validate_request(&req(b31, None)).is_ok());
}

#[test]
fn validate_request_rejects_wrong_boundary_count() {
    let bad: Vec<i64> = (0..=5).map(|i| i * DAY).collect();
    assert!(validate_request(&req(bad, None)).is_err());
}

#[test]
fn validate_request_rejects_non_increasing_boundaries() {
    let mut b = boundaries_7();
    b[3] = b[2]; // zero-width interval
    assert!(validate_request(&req(b, None)).is_err());
}

#[test]
fn validate_request_rejects_interval_over_48h() {
    let mut b = boundaries_7();
    b[1] = b[0] + 49 * 3600;
    assert!(validate_request(&req(b, None)).is_err());
}

#[test]
fn validate_request_rejects_boundary_out_of_chrono_representable_range() {
    // All 7 intervals stay well within the 48h band (exactly one day each)
    // so only the finite-range check can be responsible for the rejection;
    // the window is shifted to straddle chrono's actual max representable
    // instant rather than a hardcoded guess.
    let max_ts = chrono::DateTime::<chrono::Utc>::MAX_UTC.timestamp();
    let base = max_ts - 6 * DAY;
    let b: Vec<i64> = (0..=7).map(|i| base + i * DAY).collect();
    assert!(
        b[7] > max_ts,
        "test setup must push the last boundary out of range"
    );
    assert!(validate_request(&req(b, None)).is_err());
}

#[test]
fn validate_request_accepts_30_minute_dst_interval() {
    // Lord Howe Island: 30-minute DST offset. A day-boundary pair differing
    // by 23.5h must be accepted under the 48h sanity band (A9).
    let mut b = boundaries_7();
    b[1] = b[0] + 23 * 3600 + 1800;
    assert!(validate_request(&req(b, None)).is_ok());
}

#[test]
fn validate_request_normalizes_pubkey_to_lowercase() {
    let pk = "AB".repeat(32);
    let result = validate_request(&req(boundaries_7(), Some(pk.clone())));
    assert_eq!(result.unwrap(), Some(pk.to_lowercase()));
}

#[test]
fn validate_request_rejects_malformed_pubkey() {
    let short = "ab".repeat(10);
    assert!(validate_request(&req(boundaries_7(), Some(short))).is_err());
    let non_hex = "zz".repeat(32);
    assert!(validate_request(&req(boundaries_7(), Some(non_hex))).is_err());
}
