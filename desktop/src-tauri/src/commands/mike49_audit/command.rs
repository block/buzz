//! MIKE-49 checkpoint 1 (Desktop fixture-only audit command): the Tauri
//! command itself, plus the explicit, allowlisted `#[derive(Serialize)]`
//! report types that are the ONLY thing this module ever returns to the
//! frontend. Composes [`super::fixture`] (disposable-key event
//! construction + real `nip-am-verify` pagination/verification/coverage
//! calls) and [`super::sanitize`] (the ported Python-bridge sanitizer gate)
//! — never re-implements either.
//!
//! This command takes NO `AppState` parameter and has NO access path to
//! the app's real signing key, the archive, or `native_relay_client` — it
//! is fixture-only, simulated-data-only, by construction (see this crate's
//! own `grep` verification in the PR description). `simulated: true` /
//! `data_source: "in_memory_fixture"` are always present on the returned
//! report so the frontend (and anyone reading a captured payload) can
//! never mistake this for live data.

use std::collections::BTreeMap;

use nip_am_verify::cursor::StopReason;
use nip_am_verify::{verify_and_decrypt, SessionCoverageTracker};
use serde::Serialize;
use serde_json::{json, Map, Value};

use super::fixture::{build_empty_source, build_fixture, run_single_capped_call};
use super::sanitize::{
    is_bool, is_bool_or_none, is_bounded_str_or_none, is_finite_nonneg_number_or_none,
    is_nonempty_str, is_nonneg_int, is_nonneg_int_or_none, sanitize_kind_value, FieldSchema,
};

const VERIFY_ERROR_CODES: &[&str] = &[
    "bad_signature",
    "wrong_kind",
    "not_exactly_one_p_tag",
    "wrong_recipient",
    "not_exactly_one_agent_tag",
    "signer_tag_mismatch",
    "decrypt_failed",
    "malformed_payload",
    "empty_identifier",
    "invalid_timestamp",
    "missing_required_fields_for_cumulative",
    "invalid_cost",
];

const OBSERVE_OUTCOME_CODES: &[&str] = &[
    "accepted",
    "unsequenced",
    "duplicate_event_id",
    "same_sequence_republish",
    "sequence_gap",
    "conflicting_payload_at_same_sequence",
];

const COST_NOTE: &str = "harness estimate, never a billed charge";
const PAGINATION_NOTE: &str = "local_traversal_complete only means this run's own fixture page source had nothing further to return -- it is never proof a real relay published nothing more; this command never queries a real relay at all";
const EMPTY_PROBE_NOTE: &str = "a genuinely empty in-memory fixture page source: 0 events, exhausted on the first page fetch -- distinct from a capped/incomplete result, which has more data behind it";

const TOKENS_ALLOWED: &[(&str, FieldSchema)] = &[
    (
        "turn_input_tokens",
        FieldSchema::Validator(is_nonneg_int_or_none),
    ),
    (
        "turn_output_tokens",
        FieldSchema::Validator(is_nonneg_int_or_none),
    ),
    (
        "turn_total_tokens",
        FieldSchema::Validator(is_nonneg_int_or_none),
    ),
    (
        "cumulative_input_tokens",
        FieldSchema::Validator(is_nonneg_int_or_none),
    ),
    (
        "cumulative_output_tokens",
        FieldSchema::Validator(is_nonneg_int_or_none),
    ),
    (
        "cumulative_total_tokens",
        FieldSchema::Validator(is_nonneg_int_or_none),
    ),
    ("delta_reliable", FieldSchema::Validator(is_bool_or_none)),
];

const COST_ALLOWED: &[(&str, FieldSchema)] = &[
    (
        "turn_cost_usd",
        FieldSchema::Validator(is_finite_nonneg_number_or_none),
    ),
    (
        "cumulative_cost_usd",
        FieldSchema::Validator(is_finite_nonneg_number_or_none),
    ),
    ("note", FieldSchema::FixedSet(&[COST_NOTE])),
];

fn is_verify_error_code_or_none(v: &Value) -> bool {
    v.is_null() || matches!(v, Value::String(s) if VERIFY_ERROR_CODES.contains(&s.as_str()))
}

fn is_observe_outcome_code_or_none(v: &Value) -> bool {
    v.is_null() || matches!(v, Value::String(s) if OBSERVE_OUTCOME_CODES.contains(&s.as_str()))
}

const RECORD_ALLOWED: &[(&str, FieldSchema)] = &[
    ("kind", FieldSchema::FixedSet(&["record"])),
    ("scenario", FieldSchema::Validator(is_nonempty_str)),
    (
        "result",
        FieldSchema::FixedSet(&["verified", "verify_error"]),
    ),
    (
        "verify_error_code",
        FieldSchema::Validator(is_verify_error_code_or_none),
    ),
    (
        "observe_outcome_code",
        FieldSchema::Validator(is_observe_outcome_code_or_none),
    ),
    (
        "event_id",
        FieldSchema::Validator(super::sanitize::is_hex_id_or_none),
    ),
    (
        "signer",
        FieldSchema::Validator(super::sanitize::is_hex_id_or_none),
    ),
    ("session_id", FieldSchema::Validator(is_bounded_str_or_none)),
    ("turn_id", FieldSchema::Validator(is_bounded_str_or_none)),
    ("turn_seq", FieldSchema::Validator(is_nonneg_int_or_none)),
    ("harness", FieldSchema::Validator(is_bounded_str_or_none)),
    ("model", FieldSchema::Validator(is_bounded_str_or_none)),
    (
        "stop_reason",
        FieldSchema::Validator(is_bounded_str_or_none),
    ),
    ("tokens", FieldSchema::Nested(TOKENS_ALLOWED)),
    ("cost", FieldSchema::Nested(COST_ALLOWED)),
];

const STOP_REASON_CODES: &[&str] = &[
    "exhausted",
    "reached_boundary",
    "page_cap_reached",
    "event_cap_reached",
];

const PAGINATION_ALLOWED: &[(&str, FieldSchema)] = &[
    ("kind", FieldSchema::FixedSet(&["pagination"])),
    ("events_examined", FieldSchema::Validator(is_nonneg_int)),
    ("stop_reason", FieldSchema::FixedSet(STOP_REASON_CODES)),
    ("local_traversal_complete", FieldSchema::Validator(is_bool)),
    ("page_limit", FieldSchema::Validator(is_nonneg_int)),
    (
        "max_pages_per_call",
        FieldSchema::Validator(is_nonneg_int_or_none),
    ),
    ("note", FieldSchema::FixedSet(&[PAGINATION_NOTE])),
];

const EMPTY_PROBE_ALLOWED: &[(&str, FieldSchema)] = &[
    ("kind", FieldSchema::FixedSet(&["empty_source_probe"])),
    ("events_examined", FieldSchema::Validator(is_nonneg_int)),
    ("stop_reason", FieldSchema::FixedSet(STOP_REASON_CODES)),
    ("note", FieldSchema::FixedSet(&[EMPTY_PROBE_NOTE])),
];

const ALLOWLISTS_BY_KIND: &[(&str, &[(&str, FieldSchema)])] = &[
    ("record", RECORD_ALLOWED),
    ("pagination", PAGINATION_ALLOWED),
    ("empty_source_probe", EMPTY_PROBE_ALLOWED),
];

fn stop_reason_code(reason: StopReason) -> &'static str {
    match reason {
        StopReason::Exhausted => "exhausted",
        StopReason::ReachedBoundary => "reached_boundary",
        StopReason::PageCapReached => "page_cap_reached",
        StopReason::EventCapReached => "event_cap_reached",
    }
}

/// One `record` line's worth of data, kept in a plain map so it can be
/// converted to an intermediate `serde_json::Value`, run through
/// [`sanitize_kind_value`] like `Value`-shaped data from any other
/// producer would be, and only THEN allowed to contribute to the final
/// typed report.
#[allow(clippy::too_many_arguments)]
fn build_record_raw_value(
    scenario: &'static str,
    result: &'static str,
    verify_error_code: Option<&'static str>,
    observe_outcome_code: Option<&'static str>,
    event_id: Option<String>,
    signer: Option<String>,
    session_id: Option<String>,
    turn_id: Option<String>,
    turn_seq: Option<u64>,
    harness: Option<String>,
    model: Option<String>,
    stop_reason: Option<String>,
    tokens: Option<Value>,
    cost: Option<Value>,
) -> Value {
    let mut map = Map::new();
    map.insert("kind".into(), json!("record"));
    map.insert("scenario".into(), json!(scenario));
    map.insert("result".into(), json!(result));
    map.insert("verify_error_code".into(), json!(verify_error_code));
    map.insert("observe_outcome_code".into(), json!(observe_outcome_code));
    map.insert("event_id".into(), json!(event_id));
    map.insert("signer".into(), json!(signer));
    map.insert("session_id".into(), json!(session_id));
    map.insert("turn_id".into(), json!(turn_id));
    map.insert("turn_seq".into(), json!(turn_seq));
    map.insert("harness".into(), json!(harness));
    map.insert("model".into(), json!(model));
    map.insert("stop_reason".into(), json!(stop_reason));
    if let Some(t) = tokens {
        map.insert("tokens".into(), t);
    }
    if let Some(c) = cost {
        map.insert("cost".into(), c);
    }
    Value::Object(map)
}

// ---------------------------------------------------------------------------
// Explicit, allowlisted report types -- the ONLY thing this command returns.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mike49TokenCounts {
    pub turn_input_tokens: Option<u64>,
    pub turn_output_tokens: Option<u64>,
    pub turn_total_tokens: Option<u64>,
    pub cumulative_input_tokens: Option<u64>,
    pub cumulative_output_tokens: Option<u64>,
    pub cumulative_total_tokens: Option<u64>,
    pub delta_reliable: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mike49Cost {
    pub turn_cost_usd: Option<f64>,
    pub cumulative_cost_usd: Option<f64>,
    pub note: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mike49Record {
    pub scenario: &'static str,
    /// `"verified"` | `"verify_error"` -- never anything else.
    pub result: &'static str,
    pub verify_error_code: Option<&'static str>,
    pub observe_outcome_code: Option<&'static str>,
    pub event_id: Option<String>,
    pub signer: Option<String>,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub turn_seq: Option<u64>,
    pub harness: Option<String>,
    pub model: Option<String>,
    pub stop_reason: Option<String>,
    /// Kept nested and SEPARATE from `cost` and from every identity/coverage
    /// field above -- signer identity, sequence shape, and billing estimate
    /// are three different claims (mirrors `pipeline_demo.rs`'s own
    /// discipline).
    pub tokens: Option<Mike49TokenCounts>,
    pub cost: Option<Mike49Cost>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mike49PaginationSummary {
    pub events_examined: u64,
    /// `"exhausted"` | `"reached_boundary"` | `"page_cap_reached"` |
    /// `"event_cap_reached"`.
    pub stop_reason: &'static str,
    /// `true` only when `stop_reason` is `"exhausted"` or
    /// `"reached_boundary"` -- this fixture's main run is deliberately
    /// capped, so this is `false` by design, proving the UI's
    /// capped/incomplete state against real (if small) pagination data.
    pub local_traversal_complete: bool,
    pub page_limit: u64,
    pub max_pages_per_call: Option<u64>,
    pub note: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mike49EmptySourceProbe {
    pub events_examined: u64,
    pub stop_reason: &'static str,
    pub note: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mike49SanitizerSummary {
    pub blocked_count: u32,
    pub blocked_by_kind: BTreeMap<String, u32>,
    pub dropped_field_count: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mike49AuditReport {
    /// Always `true` -- this command never touches real keys, the
    /// archive, or a real relay. See this crate's own `grep` verification
    /// in the PR description for `AppState`/`archive`/`native_relay_client`.
    pub simulated: bool,
    /// Always `"in_memory_fixture"`.
    pub data_source: &'static str,
    pub records: Vec<Mike49Record>,
    pub pagination: Mike49PaginationSummary,
    pub empty_source_probe: Mike49EmptySourceProbe,
    pub sanitizer: Mike49SanitizerSummary,
}

/// Every distinct way [`mike49_run_fixture_audit`] can fail. Fixed,
/// content-free codes only -- same discipline as `nip-am-verify`'s own
/// `VerifyError`/`CursorError`, never a raw library error string or a
/// `{:?}` dump.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mike49AuditError {
    /// [`super::fixture::build_fixture`] returned `None` -- a defect in
    /// this fixture's own construction, never live/attacker input (there
    /// is none).
    FixtureBuild,
    /// The fixture's own pagination call returned a
    /// [`nip_am_verify::cursor::CursorError`] -- would only ever happen on
    /// a defect in this fixture's construction, never on live/attacker
    /// input (there is none).
    PaginationError,
    /// A record's sanitized line count did not reconcile against the
    /// number of records this run actually processed -- see
    /// [`reconcile_record_count`]. Never emits a partial/best-effort
    /// report when this trips.
    ReconciliationMismatch,
}

impl std::fmt::Display for Mike49AuditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let code = match self {
            Mike49AuditError::FixtureBuild => "fixture_build_failed",
            Mike49AuditError::PaginationError => "pagination_failed",
            Mike49AuditError::ReconciliationMismatch => "reconciliation_failed",
        };
        write!(f, "{code}")
    }
}

/// Checks that the number of records this run actually decided to include
/// in the report equals the number it attempted to process (fetched via
/// the capped pagination call, minus any the sanitizer legitimately
/// blocked). Mirrors `pipeline_demo.rs`'s own explicit, unconditionally
/// compiled (never `debug_assert!`) reconciliation check -- kept as its
/// own small, directly testable function rather than inlined, so both the
/// pass and fail paths can be exercised without needing to corrupt the
/// live fixture to do it.
fn reconcile_record_count(
    attempted: usize,
    blocked_by_sanitizer: usize,
    emitted: usize,
) -> Result<(), Mike49AuditError> {
    if attempted.saturating_sub(blocked_by_sanitizer) == emitted {
        Ok(())
    } else {
        Err(Mike49AuditError::ReconciliationMismatch)
    }
}

#[tauri::command]
pub fn mike49_run_fixture_audit() -> Result<Mike49AuditReport, Mike49AuditError> {
    let fixture = build_fixture().ok_or(Mike49AuditError::FixtureBuild)?;

    let pagination_result =
        run_single_capped_call(&fixture.source, fixture.page_limit, fixture.max_pages)
            .map_err(|_| Mike49AuditError::PaginationError)?;

    let mut tracker = SessionCoverageTracker::new();
    let mut records = Vec::new();
    let mut sanitizer_blocked_count = 0u32;
    let mut sanitizer_blocked_by_kind: BTreeMap<String, u32> = BTreeMap::new();
    let mut sanitizer_dropped_field_count = 0u32;
    let attempted = pagination_result.events.len();
    let mut blocked_records = 0usize;

    for fake_event in &pagination_result.events {
        let fixture_event = &fixture.by_id[&fake_event.id];
        let scenario = fixture_event.scenario;

        let raw = match verify_and_decrypt(&fixture_event.event, &fixture.owner) {
            Err(err) => build_record_raw_value(
                scenario,
                "verify_error",
                Some(err.code()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ),
            Ok(verified) => {
                let outcome = tracker.observe(&verified);
                let payload = verified.payload();
                let turn = payload.turn.as_ref();
                let cumulative = payload.cumulative.as_ref();
                let has_any_tokens = turn.is_some() || cumulative.is_some();
                let tokens = has_any_tokens.then(|| {
                    json!({
                        "turn_input_tokens": turn.and_then(|t| t.input_tokens),
                        "turn_output_tokens": turn.and_then(|t| t.output_tokens),
                        "turn_total_tokens": turn.and_then(|t| t.total_tokens),
                        "cumulative_input_tokens": cumulative.and_then(|c| c.input_tokens),
                        "cumulative_output_tokens": cumulative.and_then(|c| c.output_tokens),
                        "cumulative_total_tokens": cumulative.and_then(|c| c.total_tokens),
                        "delta_reliable": payload.delta_reliable,
                    })
                });
                let cost = has_any_tokens.then(|| {
                    json!({
                        "turn_cost_usd": turn.and_then(|t| t.cost_usd),
                        "cumulative_cost_usd": cumulative.and_then(|c| c.cost_usd),
                        "note": COST_NOTE,
                    })
                });
                build_record_raw_value(
                    scenario,
                    "verified",
                    None,
                    Some(outcome.code()),
                    Some(verified.event_id().to_hex()),
                    Some(verified.signer().to_hex()),
                    payload.session_id.clone(),
                    payload.turn_id.clone(),
                    payload.turn_seq,
                    Some(payload.harness.clone()),
                    payload.model.clone(),
                    payload.stop_reason.clone(),
                    tokens,
                    cost,
                )
            }
        };

        let (sanitize_result, matched_kind) = sanitize_kind_value(&raw, ALLOWLISTS_BY_KIND);
        sanitizer_dropped_field_count += sanitize_result.dropped_field_count;
        if sanitize_result.blocked_field.is_some() {
            sanitizer_blocked_count += 1;
            blocked_records += 1;
            let kind_key = matched_kind.unwrap_or("unknown").to_string();
            *sanitizer_blocked_by_kind.entry(kind_key).or_insert(0) += 1;
            continue;
        }

        // The sanitizer's PASS/FAIL decision (keep-or-drop-and-count) gates
        // whether this record survives into the report; the typed fields
        // below are still sourced from the already-typed, already-safe
        // fixture data (never re-parsed out of the sanitized JSON) -- both
        // are equally trustworthy here (this is our own fixture, not
        // attacker input), and sourcing from the typed data avoids a
        // second, redundant string round-trip for the `&'static str` code
        // fields. See this module's own doc comment.
        let raw_obj = raw
            .as_object()
            .expect("build_record_raw_value always returns an object");
        let result_str: &'static str =
            if raw_obj.get("result").and_then(Value::as_str) == Some("verified") {
                "verified"
            } else {
                "verify_error"
            };
        let verify_error_code = raw_obj
            .get("verify_error_code")
            .and_then(Value::as_str)
            .and_then(|s| VERIFY_ERROR_CODES.iter().find(|c| **c == s).copied());
        let observe_outcome_code = raw_obj
            .get("observe_outcome_code")
            .and_then(Value::as_str)
            .and_then(|s| OBSERVE_OUTCOME_CODES.iter().find(|c| **c == s).copied());
        let opt_string = |key: &str| -> Option<String> {
            raw_obj.get(key).and_then(Value::as_str).map(str::to_string)
        };
        let opt_u64 = |key: &str| -> Option<u64> { raw_obj.get(key).and_then(Value::as_u64) };
        let tokens = raw_obj
            .get("tokens")
            .and_then(Value::as_object)
            .map(|t| Mike49TokenCounts {
                turn_input_tokens: t.get("turn_input_tokens").and_then(Value::as_u64),
                turn_output_tokens: t.get("turn_output_tokens").and_then(Value::as_u64),
                turn_total_tokens: t.get("turn_total_tokens").and_then(Value::as_u64),
                cumulative_input_tokens: t.get("cumulative_input_tokens").and_then(Value::as_u64),
                cumulative_output_tokens: t.get("cumulative_output_tokens").and_then(Value::as_u64),
                cumulative_total_tokens: t.get("cumulative_total_tokens").and_then(Value::as_u64),
                delta_reliable: t.get("delta_reliable").and_then(Value::as_bool),
            });
        let cost = raw_obj
            .get("cost")
            .and_then(Value::as_object)
            .map(|c| Mike49Cost {
                turn_cost_usd: c.get("turn_cost_usd").and_then(Value::as_f64),
                cumulative_cost_usd: c.get("cumulative_cost_usd").and_then(Value::as_f64),
                note: COST_NOTE,
            });

        records.push(Mike49Record {
            scenario,
            result: result_str,
            verify_error_code,
            observe_outcome_code,
            event_id: opt_string("event_id"),
            signer: opt_string("signer"),
            session_id: opt_string("session_id"),
            turn_id: opt_string("turn_id"),
            turn_seq: opt_u64("turn_seq"),
            harness: opt_string("harness"),
            model: opt_string("model"),
            stop_reason: opt_string("stop_reason"),
            tokens,
            cost,
        });
    }

    reconcile_record_count(attempted, blocked_records, records.len())?;

    let pagination_raw = json!({
        "kind": "pagination",
        "events_examined": pagination_result.events.len() as u64,
        "stop_reason": stop_reason_code(pagination_result.stop_reason),
        "local_traversal_complete": matches!(
            pagination_result.stop_reason,
            StopReason::Exhausted | StopReason::ReachedBoundary
        ),
        "page_limit": fixture.page_limit as u64,
        "max_pages_per_call": fixture.max_pages as u64,
        "note": PAGINATION_NOTE,
    });
    let (pagination_sanitized, _pagination_kind) =
        sanitize_kind_value(&pagination_raw, ALLOWLISTS_BY_KIND);
    if pagination_sanitized.blocked_field.is_some() {
        // Would only happen on a defect in this module's own construction —
        // never on live input, since there is none. Fail loudly rather
        // than silently degrade the report's own traversal-completeness
        // claim; the run's counters are discarded along with the error, so
        // there is nothing further to bookkeep on this path.
        return Err(Mike49AuditError::ReconciliationMismatch);
    }
    sanitizer_dropped_field_count += pagination_sanitized.dropped_field_count;

    let empty_result = run_single_capped_call(&build_empty_source(), 5, 1)
        .map_err(|_| Mike49AuditError::PaginationError)?;
    let empty_probe_raw = json!({
        "kind": "empty_source_probe",
        "events_examined": empty_result.events.len() as u64,
        "stop_reason": stop_reason_code(empty_result.stop_reason),
        "note": EMPTY_PROBE_NOTE,
    });
    let (empty_probe_sanitized, _empty_probe_kind) =
        sanitize_kind_value(&empty_probe_raw, ALLOWLISTS_BY_KIND);
    if empty_probe_sanitized.blocked_field.is_some() {
        return Err(Mike49AuditError::ReconciliationMismatch);
    }
    sanitizer_dropped_field_count += empty_probe_sanitized.dropped_field_count;

    Ok(Mike49AuditReport {
        simulated: true,
        data_source: "in_memory_fixture",
        records,
        pagination: Mike49PaginationSummary {
            events_examined: pagination_result.events.len() as u64,
            stop_reason: stop_reason_code(pagination_result.stop_reason),
            local_traversal_complete: matches!(
                pagination_result.stop_reason,
                StopReason::Exhausted | StopReason::ReachedBoundary
            ),
            page_limit: fixture.page_limit as u64,
            max_pages_per_call: Some(fixture.max_pages as u64),
            note: PAGINATION_NOTE,
        },
        empty_source_probe: Mike49EmptySourceProbe {
            events_examined: empty_result.events.len() as u64,
            stop_reason: stop_reason_code(empty_result.stop_reason),
            note: EMPTY_PROBE_NOTE,
        },
        sanitizer: Mike49SanitizerSummary {
            blocked_count: sanitizer_blocked_count,
            blocked_by_kind: sanitizer_blocked_by_kind,
            dropped_field_count: sanitizer_dropped_field_count,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_normal_run_produces_a_report_with_two_records_and_a_capped_pagination() {
        let report = mike49_run_fixture_audit().expect("fixture run never fails");
        assert!(report.simulated);
        assert_eq!(report.data_source, "in_memory_fixture");
        assert_eq!(report.records.len(), 2);
        assert!(!report.pagination.local_traversal_complete);
        assert_eq!(report.pagination.stop_reason, "page_cap_reached");
        assert_eq!(report.empty_source_probe.events_examined, 0);
        assert_eq!(report.empty_source_probe.stop_reason, "exhausted");
        assert_eq!(report.sanitizer.blocked_count, 0);
    }

    #[test]
    fn the_report_contains_one_verified_and_one_verify_error_record() {
        let report = mike49_run_fixture_audit().expect("fixture run never fails");
        let verified = report
            .records
            .iter()
            .filter(|r| r.result == "verified")
            .count();
        let errors = report
            .records
            .iter()
            .filter(|r| r.result == "verify_error")
            .count();
        assert_eq!(verified, 1);
        assert_eq!(errors, 1);
        let error_record = report
            .records
            .iter()
            .find(|r| r.result == "verify_error")
            .unwrap();
        assert_eq!(error_record.verify_error_code, Some("wrong_recipient"));
        let verified_record = report
            .records
            .iter()
            .find(|r| r.result == "verified")
            .unwrap();
        assert_eq!(verified_record.observe_outcome_code, Some("accepted"));
        assert!(verified_record.tokens.is_some());
        assert!(verified_record.cost.is_some());
    }

    #[test]
    fn reconcile_record_count_passes_when_counts_match() {
        assert_eq!(reconcile_record_count(2, 0, 2), Ok(()));
        assert_eq!(reconcile_record_count(3, 1, 2), Ok(()));
    }

    #[test]
    fn reconcile_record_count_fails_when_counts_diverge() {
        assert_eq!(
            reconcile_record_count(2, 0, 1),
            Err(Mike49AuditError::ReconciliationMismatch)
        );
    }

    #[test]
    fn audit_error_display_is_fixed_and_content_free() {
        assert_eq!(
            Mike49AuditError::PaginationError.to_string(),
            "pagination_failed"
        );
        assert_eq!(
            Mike49AuditError::ReconciliationMismatch.to_string(),
            "reconciliation_failed"
        );
    }

    // ---- sanitizer parity against the pinned Python reference -------------
    //
    // `RECORD_ALLOWED` above is a Rust port of buzz-auditor@83db37a's
    // `nip_am_pipeline_bridge.RECORD_ALLOWED_FIELDS` (see this crate's own
    // module doc comment on `sanitize.rs`) -- the same nested `tokens`/
    // `cost` groups, the same credential-scan-before-format-check ordering,
    // the same three-case discipline (drop+count / downgrade-to-unknown /
    // block-whole-line). `pagination`/`empty_source_probe` are NOT ported
    // from any Python module -- they are new kinds this checkpoint invented
    // because this command runs one capped in-process call, not
    // `pipeline_demo`'s multi-call `query_trace`/`run_summary`/
    // `session_coverage` sequence -- so parity is asserted for `record`
    // only, the schema actually reused.
    //
    // `testdata/sanitizer_parity_examples.json` is fed to BOTH sides:
    // `sanitizer_parity_expected_python_output.json` is the frozen,
    // captured-verbatim result of running the real
    // `collector.nip_am_pipeline_bridge.sanitize_pipeline_line` from a git
    // worktree of cccareers/buzz-auditor pinned to the exact commit in
    // `../../NIP_AM_VERIFY_PINNED_REV` (83db37a6f8fec5663227ceeb439b21c30d5d19ac) --
    // not hand-written, not "similarly named" assertions.
    //
    // One scenario, `related_event_id_present`, is a KNOWN, documented
    // divergence rather than a parity failure: the Python allowlist
    // validates a `related_event_id` field that this command's own fixture
    // producer (`build_record_raw_value`) never emits, so `RECORD_ALLOWED`
    // omits it -- Rust drops+counts it as unrecognized where Python
    // validates and keeps it. This is the strictly safer direction (a
    // narrower allowlist can only under-accept, never over-trust), so it is
    // asserted explicitly here, not silently excluded from the comparison.
    #[test]
    fn record_sanitizer_matches_the_pinned_python_reference_on_shared_examples() {
        let examples: Vec<Value> =
            serde_json::from_str(include_str!("testdata/sanitizer_parity_examples.json"))
                .expect("fixture examples are valid JSON");
        let expected: Vec<Value> = serde_json::from_str(include_str!(
            "testdata/sanitizer_parity_expected_python_output.json"
        ))
        .expect("frozen python output is valid JSON");
        assert_eq!(
            examples.len(),
            expected.len(),
            "one frozen Python result per shared example"
        );

        for (example, expected) in examples.iter().zip(expected.iter()) {
            let scenario = example
                .get("scenario")
                .and_then(Value::as_str)
                .expect("every example is labeled");
            let obj = example.as_object().expect("every example is a JSON object");

            let result = super::super::sanitize::sanitize_fields(obj, RECORD_ALLOWED);
            let expected_status = expected["status"].as_str().unwrap();
            let expected_blocked_field = expected["blocked_field"].as_str();
            let expected_invalid_fields: Vec<&str> = expected["invalid_fields"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();

            assert_eq!(
                result.blocked_field.is_some(),
                expected_status == "blocked",
                "scenario {scenario}: emitted/blocked status must match Python"
            );
            assert_eq!(
                result.blocked_field.as_deref(),
                expected_blocked_field,
                "scenario {scenario}: blocked_field must match Python (same dotted-path convention)"
            );

            let mut actual_invalid: Vec<&str> =
                result.invalid_fields.iter().map(String::as_str).collect();
            actual_invalid.sort_unstable();
            let mut expected_invalid = expected_invalid_fields.clone();
            expected_invalid.sort_unstable();

            if scenario == "related_event_id_present" {
                // Documented divergence (see test doc comment): Rust drops
                // the field Python validates, so dropped_field_count and
                // the emitted line's key set differ by exactly that one
                // field -- everything else about the line must still match.
                assert_eq!(result.dropped_field_count, 1);
                assert_eq!(expected["dropped_field_count"].as_u64(), Some(0));
                assert!(!result.output.contains_key("related_event_id"));
                assert_eq!(actual_invalid, expected_invalid);
                continue;
            }

            assert_eq!(
                result.dropped_field_count as u64,
                expected["dropped_field_count"].as_u64().unwrap(),
                "scenario {scenario}: dropped_field_count must match Python"
            );
            assert_eq!(
                actual_invalid, expected_invalid,
                "scenario {scenario}: invalid_fields must match Python"
            );

            if expected_status == "emitted" {
                let expected_line = expected["line"]
                    .as_object()
                    .expect("emitted scenarios carry a line object");
                assert_eq!(
                    &result.output, expected_line,
                    "scenario {scenario}: emitted line must match Python field-for-field"
                );
            } else {
                assert!(
                    result.output.is_empty(),
                    "scenario {scenario}: a blocked line's output must be empty"
                );
            }
        }
    }
}
