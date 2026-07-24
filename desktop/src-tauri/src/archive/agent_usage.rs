//! Pure NIP-AM usage accounting: request validation, wire types, and the
//! per-field cumulative/direct-fallback ladder.
//!
//! No Tauri or filesystem dependency — every function here takes already
//! loaded rows (or plain values) and returns plain values. The caller
//! (`mod.rs`'s `get_agent_usage_series` command, Phase 2) owns SQLite access
//! (`metric_store.rs`) and glues the two together: load window rows, compute
//! [`window_probe_keys`], load those exact-key rows, then call
//! [`compute_series`].
//!
//! Accounting contract (Rev 3, frozen plan + amendments A1/A2/A4/A9/A11–A13):
//! see `docs/nips/NIP-AM.md:119-160` for the NIP itself.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::metric_store::AgentMetricIndexRow;

// ── Request ──────────────────────────────────────────────────────────────────

/// Request for [`compute_series`]'s caller. `bucket_boundaries` are exact
/// local-midnight Unix-second boundaries built by the frontend (inclusive
/// start / exclusive end per adjacent pair): 8 entries = 7 buckets, 31
/// entries = 30 buckets.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentUsageSeriesRequest {
    pub bucket_boundaries: Vec<i64>,
    pub agent_pubkey: Option<String>,
}

/// Widest interval NIP-AM query validation admits (A9): wide enough to admit
/// every real civil-day transition (ordinary DST, 30-minute-offset zones,
/// historical calendar skips) while still rejecting arbitrary bins.
const MAX_INTERVAL_SECS: i64 = 48 * 3600;

/// Validate a request per the frozen contract + A9 (drops the 23–25h band
/// for a `> 0 && <= 48h` sanity band) and A13 pubkey normalization.
///
/// Fails closed before any SQLite work. Returns the normalized (lowercased)
/// agent pubkey, if one was supplied.
pub(super) fn validate_request(req: &AgentUsageSeriesRequest) -> Result<Option<String>, String> {
    let n = req.bucket_boundaries.len();
    if n != 8 && n != 31 {
        return Err(format!(
            "bucket_boundaries must have exactly 8 or 31 entries, got {n}"
        ));
    }

    for i in 0..n - 1 {
        let (a, b) = (req.bucket_boundaries[i], req.bucket_boundaries[i + 1]);
        if b <= a {
            return Err(format!(
                "bucket_boundaries must be strictly increasing (index {i}: {a} >= {b})"
            ));
        }
        let interval = b - a;
        if interval > MAX_INTERVAL_SECS {
            return Err(format!(
                "bucket_boundaries interval at index {i} is {interval}s, exceeds {MAX_INTERVAL_SECS}s"
            ));
        }
    }

    // Finite Unix-second bounds: must be representable as an RFC 3339
    // instant (chrono's timestamp range), independent of local timezone.
    for &t in &req.bucket_boundaries {
        if chrono::DateTime::from_timestamp(t, 0).is_none() {
            return Err(format!("bucket boundary {t} is out of representable range"));
        }
    }

    let normalized_pubkey = match &req.agent_pubkey {
        None => None,
        Some(pk) => {
            if pk.len() != 64 || !pk.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err("agent_pubkey must be exactly 64 hex characters".to_string());
            }
            Some(pk.to_lowercase())
        }
    };

    Ok(normalized_pubkey)
}

// ── Wire types ───────────────────────────────────────────────────────────────

/// Per-field completeness (A2): `value: null` means no known increment in
/// scope; `incomplete: true` on a non-null value means the value is a
/// reported lower bound, not full coverage.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageField {
    pub value: Option<String>,
    pub incomplete: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CostField {
    pub value: Option<f64>,
    pub incomplete: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportedUsage {
    pub input_tokens: UsageField,
    pub output_tokens: UsageField,
    pub total_tokens: UsageField,
    pub estimated_cost_usd: CostField,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesBucket {
    pub start: i64,
    pub end: i64,
    pub usage: ReportedUsage,
    pub report_count: i64,
    pub has_unknown_usage: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    pub model: Option<String>,
    pub usage: ReportedUsage,
    pub report_count: i64,
    pub has_unknown_usage: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentUsage {
    pub agent_pubkey: String,
    pub usage: ReportedUsage,
    pub buckets: Vec<SeriesBucket>,
    pub models: Vec<ModelUsage>,
    pub report_count: i64,
    pub has_unknown_usage: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Coverage {
    pub first_archived_at: Option<i64>,
    pub last_archived_at: Option<i64>,
    pub first_reported_at: Option<i64>,
    pub last_reported_at: Option<i64>,
    pub report_count: i64,
    pub invalid_report_count: i64,
    pub has_unknown_usage: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentUsageSeries {
    pub collection_enabled: bool,
    pub buckets: Vec<SeriesBucket>,
    pub agents: Vec<AgentUsage>,
    pub coverage: Coverage,
    /// A13: `null` when the request had no `agentPubkey` filter; otherwise
    /// `true` iff at least one surviving `agent_metric_index` row (either
    /// `parse_status`) exists for that author, with no bucket-boundary
    /// restriction. Callers compute this (DB access) and pass it through.
    pub has_archived_evidence: Option<bool>,
}

// ── Per-event field ladder (A1, A4, A11, A12) ───────────────────────────────

#[derive(Debug, Clone, Copy)]
enum FieldValue<T> {
    Known(T),
    Unknown,
}

struct EventOutcome {
    input: FieldValue<u64>,
    output: FieldValue<u64>,
    total: FieldValue<u64>,
    cost: FieldValue<f64>,
}

/// Per-field ladder for token counters (A1): adjacent nondecreasing cumulative
/// pair → diff; adjacent decreasing pair → unknown, terminal, no fallback;
/// no usable baseline for this field → `deltaReliable` direct value or
/// unknown.
fn ladder_token(
    baseline_cumulative: Option<u64>,
    current_cumulative: Option<u64>,
    current_turn: Option<u64>,
    delta_reliable: bool,
) -> FieldValue<u64> {
    if let (Some(prev), Some(cur)) = (baseline_cumulative, current_cumulative) {
        return if cur >= prev {
            FieldValue::Known(cur - prev)
        } else {
            FieldValue::Unknown
        };
    }
    if delta_reliable {
        if let Some(v) = current_turn {
            return FieldValue::Known(v);
        }
    }
    FieldValue::Unknown
}

/// Same ladder for `costUsd` (f64).
fn ladder_cost(
    baseline_cumulative: Option<f64>,
    current_cumulative: Option<f64>,
    current_turn: Option<f64>,
    delta_reliable: bool,
) -> FieldValue<f64> {
    if let (Some(prev), Some(cur)) = (baseline_cumulative, current_cumulative) {
        return if cur >= prev {
            FieldValue::Known(cur - prev)
        } else {
            FieldValue::Unknown
        };
    }
    if delta_reliable {
        if let Some(v) = current_turn {
            return FieldValue::Known(v);
        }
    }
    FieldValue::Unknown
}

/// Resolve the exact-`S-1` baseline row for `row`, or `None` if no usable
/// baseline exists (A11/A12): missing session/sequence key, duplicate row at
/// `row`'s own sequence (A4 — no cumulative delta for any row at a
/// duplicated sequence), sequence `0` (no predecessor, `checked_sub`
/// underflow), no predecessor row (gap), or duplicate rows at the
/// predecessor sequence (ambiguous baseline).
///
/// `probe_by_key` must contain, for every key queried, ALL valid rows at
/// that exact `(agent, session, turnSeq)` — used for both this baseline
/// lookup and the A4/A11 duplicate-cardinality check, which is why absence
/// of a key from the map is treated identically to an empty group.
fn resolve_baseline<'a>(
    row: &AgentMetricIndexRow,
    probe_by_key: &HashMap<(String, String, u64), Vec<&'a AgentMetricIndexRow>>,
) -> Option<&'a AgentMetricIndexRow> {
    let (agent, session, seq) = row.accounting_key()?;

    let own_group = probe_by_key.get(&(agent.clone(), session.clone(), seq))?;
    if own_group.len() > 1 {
        return None; // A4: duplicate at own sequence — no cumulative delta.
    }

    let pred_seq = seq.checked_sub(1)?; // A12: seq == 0 has no baseline.
    let pred_group = probe_by_key.get(&(agent, session, pred_seq))?;
    if pred_group.len() != 1 {
        return None; // Missing (gap) or ambiguous (duplicate) predecessor.
    }
    Some(pred_group[0])
}

fn compute_event_outcome(
    row: &AgentMetricIndexRow,
    probe_by_key: &HashMap<(String, String, u64), Vec<&AgentMetricIndexRow>>,
) -> EventOutcome {
    let baseline = resolve_baseline(row, probe_by_key);
    let delta_reliable = row.delta_reliable.unwrap_or(false);

    EventOutcome {
        input: ladder_token(
            baseline.and_then(|b| b.cumulative_input_tokens),
            row.cumulative_input_tokens,
            row.turn_input_tokens,
            delta_reliable,
        ),
        output: ladder_token(
            baseline.and_then(|b| b.cumulative_output_tokens),
            row.cumulative_output_tokens,
            row.turn_output_tokens,
            delta_reliable,
        ),
        total: ladder_token(
            baseline.and_then(|b| b.cumulative_total_tokens),
            row.cumulative_total_tokens,
            row.turn_total_tokens,
            delta_reliable,
        ),
        cost: ladder_cost(
            baseline.and_then(|b| b.cumulative_cost_usd),
            row.cumulative_cost_usd,
            row.turn_cost_usd,
            delta_reliable,
        ),
    }
}

/// The exact `(agent, session, turnSeq)` keys the caller must load via
/// `metric_store::load_rows_at_exact_keys` before calling [`compute_series`]
/// (A11): each in-window row's own key, plus its checked predecessor key
/// (`turnSeq - 1`) when one exists. Pure and DB-free so it is unit-testable
/// without SQLite.
pub(super) fn window_probe_keys(
    window_rows: &[AgentMetricIndexRow],
) -> HashSet<(String, String, u64)> {
    let mut keys = HashSet::new();
    for row in window_rows {
        if let Some((agent, session, seq)) = row.accounting_key() {
            if let Some(pred) = seq.checked_sub(1) {
                keys.insert((agent.clone(), session.clone(), pred));
            }
            keys.insert((agent, session, seq));
        }
    }
    keys
}

// ── Accumulators ─────────────────────────────────────────────────────────────

/// Sums known per-event increments with `checked_add`; an event with an
/// unknown value, or an overflow, marks the scope `incomplete` (A2) without
/// ever wrapping (overflow freezes the sum at its last valid value).
#[derive(Debug, Default, Clone)]
struct TokenAccumulator {
    value: Option<u64>,
    incomplete: bool,
    overflowed: bool,
}

impl TokenAccumulator {
    fn add(&mut self, v: FieldValue<u64>) {
        match v {
            FieldValue::Unknown => self.incomplete = true,
            FieldValue::Known(x) => {
                if self.overflowed {
                    self.incomplete = true;
                    return;
                }
                self.value = Some(match self.value {
                    None => x,
                    Some(cur) => match cur.checked_add(x) {
                        Some(sum) => sum,
                        None => {
                            self.overflowed = true;
                            self.incomplete = true;
                            cur
                        }
                    },
                });
            }
        }
    }

    fn has_unknown(&self) -> bool {
        self.incomplete
    }

    fn finish(self) -> UsageField {
        UsageField {
            value: self.value.map(|v| v.to_string()),
            incomplete: self.incomplete,
        }
    }
}

/// Same contract as [`TokenAccumulator`] for `f64` costs: "checked finite
/// addition" means a sum that would become non-finite is rejected and the
/// scope freezes at its last valid value, flagged incomplete.
#[derive(Debug, Default, Clone)]
struct CostAccumulator {
    value: Option<f64>,
    incomplete: bool,
    overflowed: bool,
}

impl CostAccumulator {
    fn add(&mut self, v: FieldValue<f64>) {
        match v {
            FieldValue::Unknown => self.incomplete = true,
            FieldValue::Known(x) => {
                if self.overflowed {
                    self.incomplete = true;
                    return;
                }
                let candidate = match self.value {
                    None => x,
                    Some(cur) => cur + x,
                };
                if candidate.is_finite() {
                    self.value = Some(candidate);
                } else {
                    self.overflowed = true;
                    self.incomplete = true;
                }
            }
        }
    }

    fn has_unknown(&self) -> bool {
        self.incomplete
    }

    fn finish(self) -> CostField {
        CostField {
            value: self.value,
            incomplete: self.incomplete,
        }
    }
}

#[derive(Debug, Default, Clone)]
struct UsageAccumulator {
    input: TokenAccumulator,
    output: TokenAccumulator,
    total: TokenAccumulator,
    cost: CostAccumulator,
}

impl UsageAccumulator {
    fn add(&mut self, outcome: &EventOutcome) {
        self.input.add(outcome.input);
        self.output.add(outcome.output);
        self.total.add(outcome.total);
        self.cost.add(outcome.cost);
    }

    fn has_unknown(&self) -> bool {
        self.input.has_unknown()
            || self.output.has_unknown()
            || self.total.has_unknown()
            || self.cost.has_unknown()
    }

    /// Raw total-token value, used only for the A2 ranking rule (sort by
    /// known `totalTokens`, unknown-total scopes listed after).
    fn total_tokens_value(&self) -> Option<u64> {
        self.total.value
    }

    fn finish(self) -> ReportedUsage {
        ReportedUsage {
            input_tokens: self.input.finish(),
            output_tokens: self.output.finish(),
            total_tokens: self.total.finish(),
            estimated_cost_usd: self.cost.finish(),
        }
    }
}

// ── Bucket assignment ────────────────────────────────────────────────────────

/// Which `[boundaries[i], boundaries[i+1])` bucket `t` falls in, or `None`
/// if outside every bucket (defensive; callers scope their row query to
/// `[boundaries[0], boundaries[last])` so this should never miss).
fn assign_bucket_index(boundaries: &[i64], t: i64) -> Option<usize> {
    (0..boundaries.len().saturating_sub(1)).find(|&i| t >= boundaries[i] && t < boundaries[i + 1])
}

// ── compute_series ───────────────────────────────────────────────────────────

/// Per-agent accumulation scope, built while walking `window_rows` once.
struct AgentScope {
    buckets: Vec<UsageAccumulator>,
    bucket_counts: Vec<i64>,
    total: UsageAccumulator,
    report_count: i64,
    models: HashMap<Option<String>, (UsageAccumulator, i64)>,
}

/// Compute the full [`AgentUsageSeries`] from already-loaded rows.
///
/// - `window_rows`: valid rows with `reported_at` in `[boundaries[0],
///   boundaries[last])` (optionally pre-filtered to one agent), from
///   `metric_store::load_window_valid_rows`.
/// - `probe_rows`: valid rows at the exact keys from [`window_probe_keys`],
///   from `metric_store::load_rows_at_exact_keys` — used for baseline
///   resolution and A4/A11 duplicate-cardinality checks, unrestricted by the
///   window.
/// - `invalid_report_count`: from `metric_store::count_invalid_rows_in_window`.
/// - `has_archived_evidence`: from `metric_store::has_archived_evidence`,
///   already resolved to `None` when the request has no `agentPubkey` filter
///   (A13) — this function does not decide that; it only carries the value.
pub(super) fn compute_series(
    window_rows: &[AgentMetricIndexRow],
    probe_rows: &[AgentMetricIndexRow],
    invalid_report_count: i64,
    boundaries: &[i64],
    has_archived_evidence: Option<bool>,
    collection_enabled: bool,
) -> AgentUsageSeries {
    let bucket_count = boundaries.len().saturating_sub(1);

    let mut probe_by_key: HashMap<(String, String, u64), Vec<&AgentMetricIndexRow>> =
        HashMap::new();
    for r in probe_rows {
        if let Some(key) = r.accounting_key() {
            probe_by_key.entry(key).or_default().push(r);
        }
    }

    let mut overall_buckets: Vec<UsageAccumulator> = (0..bucket_count)
        .map(|_| UsageAccumulator::default())
        .collect();
    let mut overall_bucket_counts: Vec<i64> = vec![0; bucket_count];

    let mut agents: HashMap<String, AgentScope> = HashMap::new();

    let mut first_reported_at: Option<i64> = None;
    let mut last_reported_at: Option<i64> = None;
    let mut first_archived_at: Option<i64> = None;
    let mut last_archived_at: Option<i64> = None;

    for row in window_rows {
        // Defensive: the loader already scopes to reported_at in-window and
        // parse_status = 'valid'; a miss here means the caller passed rows
        // it should not have, so skip rather than panic or miscount.
        let Some(reported_at) = row.reported_at else {
            continue;
        };
        let Some(bucket_idx) = assign_bucket_index(boundaries, reported_at) else {
            continue;
        };

        first_reported_at = Some(first_reported_at.map_or(reported_at, |v| v.min(reported_at)));
        last_reported_at = Some(last_reported_at.map_or(reported_at, |v| v.max(reported_at)));
        first_archived_at =
            Some(first_archived_at.map_or(row.archived_at, |v| v.min(row.archived_at)));
        last_archived_at =
            Some(last_archived_at.map_or(row.archived_at, |v| v.max(row.archived_at)));

        let outcome = compute_event_outcome(row, &probe_by_key);

        overall_buckets[bucket_idx].add(&outcome);
        overall_bucket_counts[bucket_idx] += 1;

        let scope = agents
            .entry(row.agent_pubkey.clone())
            .or_insert_with(|| AgentScope {
                buckets: (0..bucket_count)
                    .map(|_| UsageAccumulator::default())
                    .collect(),
                bucket_counts: vec![0; bucket_count],
                total: UsageAccumulator::default(),
                report_count: 0,
                models: HashMap::new(),
            });
        scope.buckets[bucket_idx].add(&outcome);
        scope.bucket_counts[bucket_idx] += 1;
        scope.total.add(&outcome);
        scope.report_count += 1;

        let model_entry = scope
            .models
            .entry(row.model.clone())
            .or_insert_with(|| (UsageAccumulator::default(), 0i64));
        model_entry.0.add(&outcome);
        model_entry.1 += 1;
    }

    let overall_report_count: i64 = overall_bucket_counts.iter().sum();

    let buckets: Vec<SeriesBucket> = overall_buckets
        .into_iter()
        .zip(overall_bucket_counts)
        .enumerate()
        .map(|(i, (acc, count))| {
            let has_unknown_usage = acc.has_unknown();
            SeriesBucket {
                start: boundaries[i],
                end: boundaries[i + 1],
                usage: acc.finish(),
                report_count: count,
                has_unknown_usage,
            }
        })
        .collect();
    let any_overall_bucket_unknown = buckets.iter().any(|b| b.has_unknown_usage);

    // Build agent rows, then apply the A2 ranking rule: known totalTokens
    // descending, unknown-total agents after, pubkey as the tiebreak/final
    // key in both groups for determinism.
    let mut agent_rows: Vec<(Option<u64>, String, AgentUsage)> = agents
        .into_iter()
        .map(|(agent_pubkey, scope)| {
            let total_tokens_value = scope.total.total_tokens_value();
            let has_unknown_usage = scope.total.has_unknown();

            let buckets: Vec<SeriesBucket> = scope
                .buckets
                .into_iter()
                .zip(scope.bucket_counts)
                .enumerate()
                .map(|(i, (acc, count))| {
                    let has_unknown_usage = acc.has_unknown();
                    SeriesBucket {
                        start: boundaries[i],
                        end: boundaries[i + 1],
                        usage: acc.finish(),
                        report_count: count,
                        has_unknown_usage,
                    }
                })
                .collect();

            let mut model_rows: Vec<(Option<u64>, Option<String>, ModelUsage)> = scope
                .models
                .into_iter()
                .map(|(model, (acc, count))| {
                    let model_total = acc.total_tokens_value();
                    let has_unknown_usage = acc.has_unknown();
                    (
                        model_total,
                        model.clone(),
                        ModelUsage {
                            model,
                            usage: acc.finish(),
                            report_count: count,
                            has_unknown_usage,
                        },
                    )
                })
                .collect();
            model_rows.sort_by(|a, b| match (a.0, b.0) {
                (Some(av), Some(bv)) => bv.cmp(&av).then_with(|| a.1.cmp(&b.1)),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.1.cmp(&b.1),
            });
            let models = model_rows.into_iter().map(|(_, _, m)| m).collect();

            (
                total_tokens_value,
                agent_pubkey.clone(),
                AgentUsage {
                    agent_pubkey,
                    usage: scope.total.finish(),
                    buckets,
                    models,
                    report_count: scope.report_count,
                    has_unknown_usage,
                },
            )
        })
        .collect();
    agent_rows.sort_by(|a, b| match (a.0, b.0) {
        (Some(av), Some(bv)) => bv.cmp(&av).then_with(|| a.1.cmp(&b.1)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.1.cmp(&b.1),
    });
    let any_agent_unknown = agent_rows.iter().any(|(_, _, a)| a.has_unknown_usage);
    let agents: Vec<AgentUsage> = agent_rows.into_iter().map(|(_, _, a)| a).collect();

    AgentUsageSeries {
        collection_enabled,
        buckets,
        agents,
        coverage: Coverage {
            first_archived_at,
            last_archived_at,
            first_reported_at,
            last_reported_at,
            report_count: overall_report_count,
            invalid_report_count,
            has_unknown_usage: any_overall_bucket_unknown
                || any_agent_unknown
                || invalid_report_count > 0,
        },
        has_archived_evidence,
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "agent_usage_tests.rs"]
mod agent_usage_tests;
