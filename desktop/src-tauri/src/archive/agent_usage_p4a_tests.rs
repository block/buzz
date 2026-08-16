//! Tests for P4a additions: cache-read/write ladder (§S-1 extension),
//! freshInputTokens derivation, and D6 comparator. Split from
//! `agent_usage_tests.rs` to keep both files under the 1 000-line ratchet.

use super::*;
use crate::archive::metric_store::{AgentMetricIndexRow, ParseStatus};

// ── Row builder ───────────────────────────────────────────────────────────────

/// Minimal valid row for P4a test scenarios.
fn row(id: &str, agent: &str, session: &str, seq: u64, reported_at: i64) -> AgentMetricIndexRow {
    AgentMetricIndexRow {
        id: id.to_string(),
        agent_pubkey: agent.to_string(),
        event_created_at: reported_at,
        archived_at: reported_at,
        reported_at: Some(reported_at),
        session_id: Some(session.to_string()),
        turn_seq: Some(seq),
        harness: None,
        model: None,
        delta_reliable: Some(true),
        turn_input_tokens: None,
        turn_output_tokens: None,
        turn_total_tokens: None,
        turn_cost_usd: None,
        turn_cache_read_tokens: None,
        cumulative_input_tokens: None,
        cumulative_output_tokens: None,
        cumulative_total_tokens: None,
        cumulative_cost_usd: None,
        cumulative_cache_read_tokens: None,
        cumulative_cache_write_tokens: None,
        turn_cache_write_tokens: None,
        pricing_authority: None,
        pricing_model: None,
        pricing_cache_class: None,
        parse_status: ParseStatus::Valid,
    }
}

/// Standard 8-boundary window covering one day, seconds since epoch.
const DAY: i64 = 86_400;
fn boundaries_7() -> Vec<i64> {
    (0..=7).map(|i| i * DAY).collect()
}

// ── Cache-read / cache-write ladder (P4a.1) ──────────────────────────────────

#[test]
fn cache_read_ladder_uses_direct_turn_value_when_no_baseline() {
    // A single event with only turn_cache_read_tokens; no cumulative, no
    // baseline → direct turn value flows through as Known.
    let r = AgentMetricIndexRow {
        turn_cache_read_tokens: Some(80),
        delta_reliable: Some(true),
        ..row("e1", "agent1", "s1", 1, 0)
    };
    let boundaries = boundaries_7();
    let rows = vec![r];
    let series = compute_series(&rows, &rows, 0, &boundaries, None, true);
    assert_eq!(
        series.agents[0].usage.cache_read_tokens,
        UsageField {
            value: Some("80".to_string()),
            incomplete: false
        }
    );
}

#[test]
fn cache_write_ladder_uses_direct_turn_value_when_no_baseline() {
    let r = AgentMetricIndexRow {
        turn_cache_write_tokens: Some(40),
        delta_reliable: Some(true),
        ..row("e1", "agent1", "s1", 1, 0)
    };
    let boundaries = boundaries_7();
    let rows = vec![r];
    let series = compute_series(&rows, &rows, 0, &boundaries, None, true);
    assert_eq!(
        series.agents[0].usage.cache_write_tokens,
        UsageField {
            value: Some("40".to_string()),
            incomplete: false
        }
    );
}

#[test]
fn omitted_cache_field_stays_unknown_not_zero_through_pipeline() {
    // Old-harness compat: a row that never reports cache_write_tokens
    // must produce incomplete=true (unknown), NOT value=Some("0").
    // The ladder returns Unknown for any absent turn/cumulative field, which
    // poisons the TokenAccumulator as incomplete — absence is NOT treated as zero.
    let r = AgentMetricIndexRow {
        turn_input_tokens: Some(100),
        delta_reliable: Some(true),
        turn_cache_write_tokens: None, // explicitly omitted
        ..row("e1", "agent1", "s1", 1, 0)
    };
    let boundaries = boundaries_7();
    let rows = vec![r];
    let series = compute_series(&rows, &rows, 0, &boundaries, None, true);
    assert_eq!(
        series.agents[0].usage.cache_write_tokens,
        UsageField {
            value: None,
            incomplete: true
        },
        "absent cache_write_tokens must produce incomplete=true (unknown), not zero"
    );
}

#[test]
fn explicit_zero_cache_field_survives_as_zero_not_absent() {
    // A row reporting cache_write = 0 (explicit confirmed-zero) must produce
    // value=Some("0"), not None.
    let r = AgentMetricIndexRow {
        turn_cache_write_tokens: Some(0),
        delta_reliable: Some(true),
        ..row("e1", "agent1", "s1", 1, 0)
    };
    let boundaries = boundaries_7();
    let rows = vec![r];
    let series = compute_series(&rows, &rows, 0, &boundaries, None, true);
    assert_eq!(
        series.agents[0].usage.cache_write_tokens,
        UsageField {
            value: Some("0".to_string()),
            incomplete: false
        },
        "explicit zero must survive as '0' not absent"
    );
}

#[test]
fn cache_read_adjacent_cumulative_preferred_over_direct() {
    // Two adjacent events with cumulative_cache_read_tokens: the second
    // event's diff (200 - 100 = 100) is used, not the direct turn value.
    let baseline = AgentMetricIndexRow {
        cumulative_cache_read_tokens: Some(100),
        delta_reliable: Some(true),
        ..row("e1", "agent1", "s1", 1, 0)
    };
    let current = AgentMetricIndexRow {
        turn_cache_read_tokens: Some(999), // direct — should NOT be used
        cumulative_cache_read_tokens: Some(200),
        delta_reliable: Some(true),
        ..row("e2", "agent1", "s1", 2, 1)
    };
    let boundaries = boundaries_7();
    let rows = vec![baseline, current];
    let series = compute_series(&rows, &rows, 0, &boundaries, None, true);
    // baseline contributes Unknown (no baseline before it, no turn value)
    // then current contributes cumulative diff = 100
    // Total = Unknown accumulation → incomplete
    assert!(series.agents[0].usage.cache_read_tokens.incomplete);
}

// ── freshInputTokens derivation (P4a.2) ──────────────────────────────────────

#[test]
fn fresh_input_is_input_minus_cache_subsets() {
    // input=100, cache_read=30, cache_write=20 → fresh=50
    let r = AgentMetricIndexRow {
        turn_input_tokens: Some(100),
        turn_cache_read_tokens: Some(30),
        turn_cache_write_tokens: Some(20),
        delta_reliable: Some(true),
        ..row("e1", "agent1", "s1", 1, 0)
    };
    let boundaries = boundaries_7();
    let rows = vec![r];
    let series = compute_series(&rows, &rows, 0, &boundaries, None, true);
    assert_eq!(
        series.agents[0].usage.fresh_input_tokens,
        UsageField {
            value: Some("50".to_string()),
            incomplete: false
        }
    );
}

#[test]
fn fresh_input_fail_closed_when_cache_fields_absent() {
    // cache_read and cache_write absent → ladder returns Unknown → accumulator
    // is incomplete → fresh_input fails closed (incomplete: true).
    // Old-harness compat: absent cache fields are unknown, not zero.
    let r = AgentMetricIndexRow {
        turn_input_tokens: Some(100),
        turn_cache_read_tokens: None,
        turn_cache_write_tokens: None,
        delta_reliable: Some(true),
        ..row("e1", "agent1", "s1", 1, 0)
    };
    let boundaries = boundaries_7();
    let rows = vec![r];
    let series = compute_series(&rows, &rows, 0, &boundaries, None, true);
    assert_eq!(
        series.agents[0].usage.fresh_input_tokens,
        UsageField {
            value: None,
            incomplete: true
        },
        "absent cache fields produce unknown fresh_input, not input-verbatim"
    );
}

#[test]
fn fresh_input_fail_closed_when_subsets_exceed_input() {
    // cache_read=60 + cache_write=60 = 120 > input=100 → incomplete
    let r = AgentMetricIndexRow {
        turn_input_tokens: Some(100),
        turn_cache_read_tokens: Some(60),
        turn_cache_write_tokens: Some(60),
        delta_reliable: Some(true),
        ..row("e1", "agent1", "s1", 1, 0)
    };
    let boundaries = boundaries_7();
    let rows = vec![r];
    let series = compute_series(&rows, &rows, 0, &boundaries, None, true);
    assert_eq!(
        series.agents[0].usage.fresh_input_tokens,
        UsageField {
            value: None,
            incomplete: true
        },
        "cacheRead+cacheWrite > input must produce incomplete fresh_input"
    );
}

#[test]
fn fresh_input_fail_closed_when_subset_sum_overflows() {
    // Both cache fields are u64::MAX/2 + 1 → their sum overflows u64 → incomplete
    let half_plus_one = u64::MAX / 2 + 1;
    let r = AgentMetricIndexRow {
        turn_input_tokens: Some(u64::MAX),
        turn_cache_read_tokens: Some(half_plus_one),
        turn_cache_write_tokens: Some(half_plus_one),
        delta_reliable: Some(true),
        ..row("e1", "agent1", "s1", 1, 0)
    };
    let boundaries = boundaries_7();
    let rows = vec![r];
    let series = compute_series(&rows, &rows, 0, &boundaries, None, true);
    assert_eq!(
        series.agents[0].usage.fresh_input_tokens,
        UsageField {
            value: None,
            incomplete: true
        },
        "cache subset sum overflow must produce incomplete fresh_input"
    );
}

#[test]
fn fresh_input_fail_closed_when_input_unknown() {
    // Unknown input (unreliable delta, no cumulative) → fresh_input incomplete
    let r = AgentMetricIndexRow {
        turn_input_tokens: Some(100),
        delta_reliable: Some(false), // unreliable, no cumulative → Unknown
        turn_cache_read_tokens: Some(10),
        turn_cache_write_tokens: Some(5),
        ..row("e1", "agent1", "s1", 1, 0)
    };
    let boundaries = boundaries_7();
    let rows = vec![r];
    let series = compute_series(&rows, &rows, 0, &boundaries, None, true);
    assert_eq!(
        series.agents[0].usage.fresh_input_tokens,
        UsageField {
            value: None,
            incomplete: true
        },
        "unknown input must make fresh_input incomplete"
    );
}

// ── D6 comparator test vector (P4a.3) ────────────────────────────────────────
//
// This vector pins the Rust sort order that the TS layer must match for
// model-breakdown rendering (P5). Each scenario documents the expected
// position in the sorted slice.

#[test]
fn d6_agent_with_total_ranks_before_input_output_only() {
    // Agent A: total=200 (known from provider)
    // Agent B: input=150, output=100 (total unknown) → sort value = 250
    // D6: A has total=200 < 250, so B ranks first (higher sort value)
    let a = AgentMetricIndexRow {
        turn_total_tokens: Some(200),
        delta_reliable: Some(true),
        ..row("e1", "agent_a", "s1", 1, 0)
    };
    let b = AgentMetricIndexRow {
        turn_input_tokens: Some(150),
        turn_output_tokens: Some(100),
        delta_reliable: Some(true),
        ..row("e2", "agent_b", "s2", 1, 1)
    };
    let boundaries = boundaries_7();
    let rows = vec![a, b];
    let series = compute_series(&rows, &rows, 0, &boundaries, None, true);
    // B: input+output = 250 > A: total = 200 → B ranks first
    assert_eq!(series.agents[0].agent_pubkey, "agent_b");
    assert_eq!(series.agents[1].agent_pubkey, "agent_a");
}

#[test]
fn d6_input_output_fallback_used_when_total_unknown() {
    // Agent A: total unknown, input=100 only (output=None) → sort value = None
    // Agent B: total unknown, input=80, output=40 → sort value = 120
    // D6: B has a sort value, A does not → B ranks first
    let a = AgentMetricIndexRow {
        turn_input_tokens: Some(100),
        delta_reliable: Some(true),
        ..row("e1", "agent_a", "s1", 1, 0)
    };
    let b = AgentMetricIndexRow {
        turn_input_tokens: Some(80),
        turn_output_tokens: Some(40),
        delta_reliable: Some(true),
        ..row("e2", "agent_b", "s2", 1, 1)
    };
    let boundaries = boundaries_7();
    let rows = vec![a, b];
    let series = compute_series(&rows, &rows, 0, &boundaries, None, true);
    assert_eq!(
        series.agents[0].agent_pubkey, "agent_b",
        "input+output fallback must rank before unknown-sort-value agent"
    );
    assert_eq!(series.agents[1].agent_pubkey, "agent_a");
}

#[test]
fn d6_model_total_ranks_before_input_output_fallback_within_agent() {
    // Same agent, two model rows:
    //   model-a: total=300 (known)
    //   model-b: input=200, output=150, total unknown → sort value = 350
    // D6: model-b sort value 350 > model-a total 300 → model-b ranks first
    let ra = AgentMetricIndexRow {
        model: Some("model-a".to_string()),
        turn_total_tokens: Some(300),
        delta_reliable: Some(true),
        ..row("e1", "agent1", "s1", 1, 0)
    };
    let rb = AgentMetricIndexRow {
        model: Some("model-b".to_string()),
        turn_input_tokens: Some(200),
        turn_output_tokens: Some(150),
        delta_reliable: Some(true),
        ..row("e2", "agent1", "s2", 1, 1)
    };
    let boundaries = boundaries_7();
    let rows = vec![ra, rb];
    let series = compute_series(&rows, &rows, 0, &boundaries, None, true);
    let models = &series.agents[0].models;
    assert_eq!(models[0].model.as_deref(), Some("model-b"));
    assert_eq!(models[1].model.as_deref(), Some("model-a"));
}

#[test]
fn d6_unknown_sort_value_ranks_last() {
    // Three agents:
    //   agent_c: total=100 (known)
    //   agent_b: input=50, output=60 (total unknown) → sort value = 110
    //   agent_a: input only, no output, no total → sort value = None (unknown)
    // Expected order: agent_b (110), agent_c (100), agent_a (None)
    let c = AgentMetricIndexRow {
        turn_total_tokens: Some(100),
        delta_reliable: Some(true),
        ..row("e1", "agent_c", "s1", 1, 0)
    };
    let b = AgentMetricIndexRow {
        turn_input_tokens: Some(50),
        turn_output_tokens: Some(60),
        delta_reliable: Some(true),
        ..row("e2", "agent_b", "s2", 1, 1)
    };
    let a = AgentMetricIndexRow {
        turn_input_tokens: Some(200), // large input but no output → no sort value
        delta_reliable: Some(true),
        ..row("e3", "agent_a", "s3", 1, 2)
    };
    let boundaries = boundaries_7();
    let rows = vec![c, b, a];
    let series = compute_series(&rows, &rows, 0, &boundaries, None, true);
    assert_eq!(series.agents[0].agent_pubkey, "agent_b");
    assert_eq!(series.agents[1].agent_pubkey, "agent_c");
    assert_eq!(series.agents[2].agent_pubkey, "agent_a");
}

// ── hasUnknownUsage propagation (P4b: Wes's CHANGES_REQUESTED fixes) ─────────

/// Wes's escape (a): `derive_fresh_input()` returns `incomplete: true` when
/// subsets exceed input, but `hasUnknownUsage` was false because the four
/// original checked fields (input, output, total, cost) were all complete.
/// This pins that `hasUnknownUsage` is true at bucket, agent, model, and
/// top-level-coverage levels whenever derivation fails.
#[test]
fn has_unknown_usage_true_when_fresh_input_derivation_fails() {
    // All four fields that the original has_unknown() checked are complete:
    // input=100, output=50, total=150, cost=0.01 — no incomplete values there.
    // But cache_read=60 + cache_write=60 = 120 > input=100 → derive fails.
    // Before the fix, has_unknown() returned false (none of the four original
    // fields were incomplete). After the fix it returns true.
    let r = AgentMetricIndexRow {
        turn_input_tokens: Some(100),
        turn_output_tokens: Some(50),
        turn_total_tokens: Some(150),
        turn_cost_usd: Some(0.01),
        turn_cache_read_tokens: Some(60),
        turn_cache_write_tokens: Some(60),
        delta_reliable: Some(true),
        ..row("e1", "agent1", "s1", 1, 0)
    };
    let boundaries = boundaries_7();
    let rows = vec![r];
    let series = compute_series(&rows, &rows, 0, &boundaries, None, true);

    // Bucket-level flag.
    assert!(
        series.buckets[0].has_unknown_usage,
        "bucket hasUnknownUsage must be true when fresh_input derivation fails"
    );
    // Agent-level flag.
    assert!(
        series.agents[0].has_unknown_usage,
        "agent hasUnknownUsage must be true when fresh_input derivation fails"
    );
    // Model-level flag.
    assert!(
        series.agents[0].models[0].has_unknown_usage,
        "model hasUnknownUsage must be true when fresh_input derivation fails"
    );
    // Top-level coverage flag.
    assert!(
        series.coverage.has_unknown_usage,
        "coverage hasUnknownUsage must be true when fresh_input derivation fails"
    );
    // Confirm the fresh_input value reflects the derivation failure.
    assert_eq!(
        series.agents[0].usage.fresh_input_tokens,
        super::UsageField {
            value: None,
            incomplete: true,
        },
        "fresh_input_tokens must be incomplete when subsets exceed input"
    );
}

/// Wes's escape (b): old harness omits cache fields → cache/fresh accumulators
/// are incomplete while `hasUnknownUsage` was false (the four original checked
/// fields were complete).  This pins that `hasUnknownUsage` is true whenever
/// cache fields are absent.
#[test]
fn has_unknown_usage_true_when_cache_fields_absent() {
    // All four fields that the original has_unknown() checked are complete:
    // input=100, output=50, total=150, cost=0.01 — no incomplete values there.
    // But cache_read and cache_write are absent (old harness) → cache
    // accumulators are incomplete → fresh_input is incomplete → has_unknown true.
    // Before the fix, has_unknown() returned false because input/output/total/cost
    // were complete. After the fix it returns true (cache accumulators are incomplete).
    let r = AgentMetricIndexRow {
        turn_input_tokens: Some(100),
        turn_output_tokens: Some(50),
        turn_total_tokens: Some(150),
        turn_cost_usd: Some(0.01),
        turn_cache_read_tokens: None,  // absent — old harness
        turn_cache_write_tokens: None, // absent — old harness
        delta_reliable: Some(true),
        ..row("e1", "agent1", "s1", 1, 0)
    };
    let boundaries = boundaries_7();
    let rows = vec![r];
    let series = compute_series(&rows, &rows, 0, &boundaries, None, true);

    // Bucket-level flag.
    assert!(
        series.buckets[0].has_unknown_usage,
        "bucket hasUnknownUsage must be true when cache fields absent"
    );
    // Agent-level flag.
    assert!(
        series.agents[0].has_unknown_usage,
        "agent hasUnknownUsage must be true when cache fields absent"
    );
    // Model-level flag.
    assert!(
        series.agents[0].models[0].has_unknown_usage,
        "model hasUnknownUsage must be true when cache fields absent"
    );
    // Top-level coverage flag.
    assert!(
        series.coverage.has_unknown_usage,
        "coverage hasUnknownUsage must be true when cache fields absent"
    );
    // Sanity: the cache and fresh_input fields are all incomplete.
    assert!(
        series.agents[0].usage.cache_read_tokens.incomplete,
        "cache_read_tokens must be incomplete when absent"
    );
    assert!(
        series.agents[0].usage.cache_write_tokens.incomplete,
        "cache_write_tokens must be incomplete when absent"
    );
    assert!(
        series.agents[0].usage.fresh_input_tokens.incomplete,
        "fresh_input_tokens must be incomplete when cache fields absent"
    );
}
