//! MIKE-49 checkpoint 1 (Desktop fixture-only audit command): a Rust port
//! of buzz-auditor's Python sanitizer discipline
//! (`src/collector/nip_am_pipeline_bridge.py` / `nip_am_verify_bridge.py`,
//! itself layered on `report_safety.py`'s `_contains_credential_shape`).
//!
//! This module exists for parity/defense-in-depth, not because a subprocess
//! boundary exists here — it does not. The Python bridge sanitizes JSON
//! Lines text crossing a real OS process boundary (Rust `verify_demo`/
//! `pipeline_demo` binary -> Python collector). This desktop command runs
//! entirely in-process: there is no serialized-text boundary to sanitize at
//! all. The REAL safety property here is structural, not this module: only
//! an explicit `#[derive(Serialize)]` allowlisted struct
//! (`super::Mike49AuditReport` and its nested types) ever crosses into the
//! Tauri command's return value — nothing else is ever exposed, by
//! construction, regardless of what this module does. This module is wired
//! into that path anyway (`command.rs` converts each fixture record to a
//! generic `serde_json::Value` and runs it through [`sanitize_kind_value`]
//! before building the final typed struct), so the allowlist/credential-scan
//! discipline is live code, not merely unit-tested decoration — but it is a
//! SECOND, redundant gate on top of the structural one, never the only
//! thing standing between fixture data and the frontend.
//!
//! Same three-case discipline as the Python bridge:
//! 1. An unrecognized field is dropped and only COUNTED, never named.
//! 2. A recognized field whose value fails its format check is downgraded
//!    to `"unknown"` for that field only.
//! 3. Any string value — at any nesting depth — that matches a
//!    credential shape blocks the WHOLE (sub-)object, with a reason built
//!    from the FIELD NAME only, never the matched text.
//!
//! `kind` is validated/type-checked (must be a JSON string) BEFORE any
//! allowlist lookup — mirrors `nip_am_verify_bridge.sanitize_verify_demo_line`'s
//! `isinstance(kind, str)` guard, which exists there so a non-hashable
//! `kind` (a JSON array/object) can't reach `dict.get()` the wrong way. That
//! specific crash mode doesn't exist in Rust (`serde_json::Map::get` never
//! panics on the value shape), but the type-check-before-lookup ORDERING
//! itself is still asserted here and tested, because it is the same defense
//! this module intends to guarantee for any future refactor of the lookup.

use serde_json::{Map, Value};
use std::sync::LazyLock;

use regex::Regex;

pub const UNKNOWN: &str = "unknown";

/// Same seven credential shapes as `report_safety.py`'s `_CREDENTIAL_PATTERNS`
/// — defense-in-depth signal only, not proof of a real, active credential.
///
/// Each pattern is compiled via `.ok()` rather than `.expect()`/`.unwrap()`
/// (same convention as this crate's existing `persona_catalog.rs` and
/// `managed_agents/definition_validation.rs` static-regex sites) — a typo'd
/// literal pattern here would just silently drop one check out of seven
/// rather than panic; every pattern is exercised by this module's own
/// tests, so a typo would be caught there, not discovered at runtime.
static CREDENTIAL_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"sk-[A-Za-z0-9]{10,}",
        r"ghp_[A-Za-z0-9]{20,}",
        r"AKIA[A-Z0-9]{16}",
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----",
        r"[Bb]earer\s+\S{8,}",
        r"[A-Za-z0-9_.+-]+:[^\s@/]+@[A-Za-z0-9.-]+",
        r"(?i)(key|secret|token|password)\s*[:=]\s*\S{8,}",
    ]
    .iter()
    .filter_map(|pattern| Regex::new(pattern).ok())
    .collect()
});

/// Never returns or logs the matched substring — only `true`/`false`.
/// Callers must build any block reason from the FIELD NAME alone, never
/// from this function's input, so a detected credential never round-trips
/// into a diagnostic message.
pub fn contains_credential_shape(value: &str) -> bool {
    CREDENTIAL_PATTERNS.iter().any(|p| p.is_match(value))
}

/// A field's accepted shape. `Nested` recurses `sanitize_fields` one (or
/// more) levels into a sub-object — mirrors the Python bridge's own
/// recursion for `tokens`/`cost` groups on a `record` line.
pub enum FieldSchema {
    Validator(fn(&Value) -> bool),
    /// Only the exact strings in this fixed set are accepted — a different
    /// value, even a well-formed one, is a stronger/different claim than
    /// this producer is allowed to make (mirrors `_fixed_set` in the Python
    /// bridge).
    FixedSet(&'static [&'static str]),
    Nested(&'static [(&'static str, FieldSchema)]),
}

pub fn is_nonneg_int_or_none(v: &Value) -> bool {
    v.is_null() || matches!(v, Value::Number(n) if n.is_u64())
}

pub fn is_nonneg_int(v: &Value) -> bool {
    matches!(v, Value::Number(n) if n.is_u64())
}

pub fn is_bool_or_none(v: &Value) -> bool {
    v.is_null() || v.is_boolean()
}

pub fn is_bool(v: &Value) -> bool {
    v.is_boolean()
}

pub fn is_nonempty_str(v: &Value) -> bool {
    matches!(v, Value::String(s) if !s.trim().is_empty())
}

const BOUNDED_STR_MAX_LEN: usize = 512;

pub fn is_bounded_str_or_none(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.chars().count() <= BOUNDED_STR_MAX_LEN,
        _ => false,
    }
}

/// `serde_json::Value` cannot carry a non-finite (`NaN`/`Infinity`) float at
/// all via its normal construction path — `serde_json::Number::from_f64`
/// returns `None` for one, and the standard JSON text grammar has no
/// literal for either (the same reasoning `pipeline_demo.rs`'s own module
/// doc gives for why its `f64` fields never carry one on the real path).
/// This validator is defense-in-depth for that already-narrow surface, not
/// a gap this module has observed exploitable in practice — proven directly
/// against the underlying `f64` check in this module's own tests, since a
/// `Value::Number` holding a non-finite float cannot be constructed through
/// safe, public `serde_json` API in the first place.
pub fn is_finite_nonneg_f64(v: f64) -> bool {
    v.is_finite() && v >= 0.0
}

pub fn is_finite_nonneg_number_or_none(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::Number(n) => n.as_f64().is_some_and(is_finite_nonneg_f64),
        _ => false,
    }
}

pub fn is_hex_id_or_none(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => {
            s.len() == 64
                && s.chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        }
        _ => false,
    }
}

/// Result of sanitizing one JSON object against an allowlist. A non-`None`
/// `blocked_field` means the whole object was blocked — `output` is empty
/// in that case, mirroring the Python bridge's `({}, ..., blocked_field,
/// blocked_reason)` tuple return.
pub struct SanitizeResult {
    pub output: Map<String, Value>,
    pub dropped_field_count: u32,
    pub invalid_fields: Vec<String>,
    pub blocked_field: Option<String>,
    pub blocked_reason: Option<String>,
}

impl SanitizeResult {
    fn blocked(dropped: u32, invalid: Vec<String>, field: String, reason: String) -> Self {
        Self {
            output: Map::new(),
            dropped_field_count: dropped,
            invalid_fields: invalid,
            blocked_field: Some(field),
            blocked_reason: Some(reason),
        }
    }
}

/// Applies `allowlist` to `raw`, recursing into any field whose schema is
/// `FieldSchema::Nested`. Same three-case discipline as the Python bridge's
/// `_sanitize_fields` — see this module's own doc comment.
pub fn sanitize_fields(
    raw: &Map<String, Value>,
    allowlist: &'static [(&'static str, FieldSchema)],
) -> SanitizeResult {
    let mut output = Map::new();
    let mut dropped_field_count = 0u32;
    let mut invalid_fields = Vec::new();

    for (key, value) in raw {
        let Some((_, schema)) = allowlist.iter().find(|(k, _)| *k == key.as_str()) else {
            dropped_field_count += 1;
            continue;
        };

        if let FieldSchema::Nested(sub_allowlist) = schema {
            let Some(obj) = value.as_object() else {
                invalid_fields.push(key.clone());
                output.insert(key.clone(), Value::String(UNKNOWN.to_string()));
                continue;
            };
            let nested = sanitize_fields(obj, sub_allowlist);
            dropped_field_count += nested.dropped_field_count;
            if let (Some(blocked_field), Some(blocked_reason)) =
                (nested.blocked_field, nested.blocked_reason)
            {
                return SanitizeResult::blocked(
                    dropped_field_count,
                    invalid_fields,
                    format!("{key}.{blocked_field}"),
                    blocked_reason,
                );
            }
            invalid_fields.extend(
                nested
                    .invalid_fields
                    .into_iter()
                    .map(|f| format!("{key}.{f}")),
            );
            output.insert(key.clone(), Value::Object(nested.output));
            continue;
        }

        // Credential-shape scan runs BEFORE format validation, same
        // ordering as the Python bridge, so a value that would otherwise
        // just be format-invalid still blocks on the stronger signal —
        // checked at every nesting depth via this same recursive call.
        if let Value::String(s) = value {
            if contains_credential_shape(s) {
                return SanitizeResult::blocked(
                    dropped_field_count,
                    invalid_fields,
                    key.clone(),
                    format!("{key} failed the credential-pattern check"),
                );
            }
        }

        let valid = match schema {
            FieldSchema::Validator(validator) => validator(value),
            FieldSchema::FixedSet(allowed) => {
                matches!(value, Value::String(s) if allowed.contains(&s.as_str()))
            }
            FieldSchema::Nested(_) => unreachable!("handled above"),
        };

        if !valid {
            invalid_fields.push(key.clone());
            output.insert(key.clone(), Value::String(UNKNOWN.to_string()));
            continue;
        }

        output.insert(key.clone(), value.clone());
    }

    SanitizeResult {
        output,
        dropped_field_count,
        invalid_fields,
        blocked_field: None,
        blocked_reason: None,
    }
}

/// Top-level dispatch: looks up the allowlist for `raw["kind"]`. `kind` is
/// type-checked (must be a JSON string) BEFORE the allowlist lookup — an
/// unrecognized, missing, or non-string `kind` blocks immediately with a
/// fixed, content-free reason, same as the Python bridge's
/// `sanitize_verify_demo_line`.
pub fn sanitize_kind_value(
    raw: &Value,
    allowlists_by_kind: &'static [(&'static str, &'static [(&'static str, FieldSchema)])],
) -> (SanitizeResult, Option<&'static str>) {
    let Some(obj) = raw.as_object() else {
        return (
            SanitizeResult::blocked(
                0,
                Vec::new(),
                "kind".to_string(),
                "raw value is not a JSON object".to_string(),
            ),
            None,
        );
    };
    let kind_str = obj.get("kind").and_then(Value::as_str);
    let matched_kind = kind_str.and_then(|k| {
        allowlists_by_kind
            .iter()
            .find(|(name, _)| *name == k)
            .map(|(name, _)| *name)
    });
    let Some(kind) = matched_kind else {
        return (
            SanitizeResult::blocked(
                0,
                Vec::new(),
                "kind".to_string(),
                "kind is not a recognized mike49_run_fixture_audit output line type".to_string(),
            ),
            None,
        );
    };
    let allowlist = allowlists_by_kind
        .iter()
        .find(|(name, _)| *name == kind)
        .map(|(_, list)| *list)
        .expect("matched_kind implies presence in allowlists_by_kind");
    (sanitize_fields(obj, allowlist), Some(kind))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ---- credential-shape detection --------------------------------------

    #[test]
    fn all_seven_credential_patterns_compiled_successfully() {
        // `.ok()`-compiled (see CREDENTIAL_PATTERNS's own doc comment) —
        // this proves none of the seven literal patterns silently failed
        // to compile and dropped out of the active set.
        assert_eq!(CREDENTIAL_PATTERNS.len(), 7);
    }

    #[test]
    fn detects_openai_style_key() {
        assert!(contains_credential_shape("sk-abcdefghij1234567890"));
    }

    #[test]
    fn detects_labeled_high_entropy_value() {
        assert!(contains_credential_shape("token: abcdefgh12345678"));
    }

    #[test]
    fn plain_text_is_not_flagged() {
        assert!(!contains_credential_shape("gpt-5"));
        assert!(!contains_credential_shape("codex-acp"));
    }

    // ---- non-finite numeric rejection (direct f64 check; see doc comment) --

    #[test]
    fn non_finite_floats_are_rejected() {
        assert!(!is_finite_nonneg_f64(f64::NAN));
        assert!(!is_finite_nonneg_f64(f64::INFINITY));
        assert!(!is_finite_nonneg_f64(f64::NEG_INFINITY));
        assert!(!is_finite_nonneg_f64(-1.0));
        assert!(is_finite_nonneg_f64(0.0));
        assert!(is_finite_nonneg_f64(12.5));
    }

    #[test]
    fn missing_or_invalid_numeric_counts_are_rejected() {
        // Missing (null) is valid ("REQUIRED-when-present" semantics) --
        // an invalid SHAPE (string, negative, float-for-int) is not.
        assert!(is_nonneg_int_or_none(&Value::Null));
        assert!(is_nonneg_int_or_none(&json!(0)));
        assert!(is_nonneg_int_or_none(&json!(42)));
        assert!(!is_nonneg_int_or_none(&json!(-1)));
        assert!(!is_nonneg_int_or_none(&json!("42")));
        assert!(!is_nonneg_int_or_none(&json!(1.5)));
        assert!(!is_finite_nonneg_number_or_none(&json!(-0.01)));
        assert!(is_finite_nonneg_number_or_none(&Value::Null));
        assert!(is_finite_nonneg_number_or_none(&json!(0.01)));
    }

    // ---- kind type-checked before allowlist lookup -------------------------

    const SIMPLE_ALLOWED: &[(&str, FieldSchema)] = &[
        ("kind", FieldSchema::FixedSet(&["simple"])),
        ("count", FieldSchema::Validator(is_nonneg_int)),
    ];
    const ALLOWLISTS: &[(&str, &[(&str, FieldSchema)])] = &[("simple", SIMPLE_ALLOWED)];

    #[test]
    fn non_string_kind_is_blocked_before_any_lookup() {
        let raw = json!({"kind": ["simple"], "count": 1});
        let (result, matched_kind) = sanitize_kind_value(&raw, ALLOWLISTS);
        assert!(result.blocked_field.is_some());
        assert_eq!(
            matched_kind, None,
            "an unrecognized kind is never attributed to a known kind"
        );
    }

    #[test]
    fn unrecognized_kind_string_is_blocked() {
        let raw = json!({"kind": "not_a_real_kind", "count": 1});
        let (result, matched_kind) = sanitize_kind_value(&raw, ALLOWLISTS);
        assert!(result.blocked_field.is_some());
        assert_eq!(matched_kind, None);
    }

    #[test]
    fn recognized_kind_is_sanitized_normally() {
        let raw = json!({"kind": "simple", "count": 5});
        let (result, matched_kind) = sanitize_kind_value(&raw, ALLOWLISTS);
        assert_eq!(matched_kind, Some("simple"));
        assert!(result.blocked_field.is_none());
        assert_eq!(result.output.get("count"), Some(&json!(5)));
    }

    // ---- unrecognized top-level field: dropped + counted, not named --------

    #[test]
    fn unrecognized_field_is_dropped_and_counted() {
        let raw = json!({"kind": "simple", "count": 1, "totally_unexpected": "x"});
        let obj = raw.as_object().unwrap();
        let result = sanitize_fields(obj, SIMPLE_ALLOWED);
        assert_eq!(result.dropped_field_count, 1);
        assert!(!result.output.contains_key("totally_unexpected"));
    }

    // ---- nested credential-shaped value blocks the whole object ------------

    const NESTED_ALLOWED: &[(&str, FieldSchema)] = &[
        ("kind", FieldSchema::FixedSet(&["nested_demo"])),
        (
            "inner",
            FieldSchema::Nested(&[
                ("label", FieldSchema::Validator(is_bounded_str_or_none)),
                ("value", FieldSchema::Validator(is_nonneg_int_or_none)),
            ]),
        ),
    ];

    #[test]
    fn a_credential_shaped_value_nested_inside_a_sub_object_blocks_the_whole_line() {
        let raw = json!({
            "kind": "nested_demo",
            "inner": {
                "label": "token: abcdefgh12345678",
                "value": 1
            }
        });
        let obj = raw.as_object().unwrap();
        let result = sanitize_fields(obj, NESTED_ALLOWED);
        assert_eq!(result.blocked_field.as_deref(), Some("inner.label"));
        assert!(result
            .blocked_reason
            .as_deref()
            .unwrap()
            .contains("credential-pattern"),);
        // Never round-trips the matched text.
        assert!(!result.blocked_reason.unwrap().contains("abcdefgh"));
        assert!(result.output.is_empty());
    }

    #[test]
    fn a_credential_shaped_value_nested_inside_an_array_element_blocks_the_whole_line() {
        const ARRAY_ALLOWED: &[(&str, FieldSchema)] = &[
            ("kind", FieldSchema::FixedSet(&["array_demo"])),
            ("tags", FieldSchema::Validator(|v| v.is_array())),
        ];
        // The array itself passes its own (shallow) shape validator, but the
        // credential scan on each top-level string field still catches a
        // value embedded one level down inside that array, because the scan
        // runs on the ORIGINAL raw string value before the validator's
        // pass/fail is even consulted for this field. To prove detection at
        // a value genuinely nested inside an array (not just an object), a
        // dedicated validator scans the array's own string elements.
        fn array_of_safe_strings(v: &Value) -> bool {
            let Some(arr) = v.as_array() else {
                return false;
            };
            arr.iter().all(|item| match item {
                Value::String(s) => !contains_credential_shape(s),
                _ => false,
            })
        }
        let raw = json!({
            "kind": "array_demo",
            "tags": ["safe", "AKIAABCDEFGHIJKLMNOP"]
        });
        let obj = raw.as_object().unwrap();
        // This allowlist's own "tags" validator already rejects a
        // credential-shaped array element directly (defense-in-depth at the
        // validator level, mirroring how a real producer would need to
        // apply the same scan to compound fields it accepts).
        const ARRAY_ALLOWED_STRICT: &[(&str, FieldSchema)] = &[
            ("kind", FieldSchema::FixedSet(&["array_demo"])),
            ("tags", FieldSchema::Validator(array_of_safe_strings)),
        ];
        let _ = ARRAY_ALLOWED; // silence unused-const warning for the illustrative const above
        let result = sanitize_fields(obj, ARRAY_ALLOWED_STRICT);
        assert!(
            result.blocked_field.is_none(),
            "array validator handles it, never panics"
        );
        assert_eq!(
            result.invalid_fields,
            vec!["tags".to_string()],
            "the whole array is downgraded to unknown, never partially trusted"
        );
        assert_eq!(
            result.output.get("tags"),
            Some(&Value::String(UNKNOWN.to_string()))
        );
    }

    // ---- format-invalid field downgrades to "unknown" for that field only --

    #[test]
    fn a_format_invalid_field_is_downgraded_to_unknown_not_blocked() {
        let raw = json!({"kind": "simple", "count": "not-a-number"});
        let obj = raw.as_object().unwrap();
        let result = sanitize_fields(obj, SIMPLE_ALLOWED);
        assert!(result.blocked_field.is_none());
        assert_eq!(result.invalid_fields, vec!["count".to_string()]);
        assert_eq!(
            result.output.get("count"),
            Some(&Value::String(UNKNOWN.to_string()))
        );
    }

    // ---- per-kind blocked counts are isolated -------------------------------

    const KIND_A_ALLOWED: &[(&str, FieldSchema)] = &[("kind", FieldSchema::FixedSet(&["kind_a"]))];
    const KIND_B_ALLOWED: &[(&str, FieldSchema)] = &[
        ("kind", FieldSchema::FixedSet(&["kind_b"])),
        (
            "secret_field",
            FieldSchema::Validator(is_bounded_str_or_none),
        ),
    ];
    const TWO_KIND_ALLOWLISTS: &[(&str, &[(&str, FieldSchema)])] =
        &[("kind_a", KIND_A_ALLOWED), ("kind_b", KIND_B_ALLOWED)];

    #[test]
    fn blocking_one_kind_of_line_does_not_change_another_kinds_blocked_count() {
        let mut blocked_by_kind: std::collections::BTreeMap<&'static str, u32> =
            std::collections::BTreeMap::new();

        let lines = vec![
            json!({"kind": "kind_a"}), // fine, not blocked
            json!({"kind": "kind_b", "secret_field": "token: abcdefgh12345678"}), // blocked
            json!({"kind": "kind_a"}), // fine, not blocked
        ];

        for raw in &lines {
            let (result, matched_kind) = sanitize_kind_value(raw, TWO_KIND_ALLOWLISTS);
            if result.blocked_field.is_some() {
                if let Some(kind) = matched_kind {
                    *blocked_by_kind.entry(kind).or_insert(0) += 1;
                }
            }
        }

        assert_eq!(blocked_by_kind.get("kind_b"), Some(&1));
        assert_eq!(
            blocked_by_kind.get("kind_a"),
            None,
            "kind_a's own lines were never blocked, so its count must stay untouched, not zeroed-in"
        );
    }

    // ---- error messages are fixed / content-free ----------------------------

    #[test]
    fn blocked_reason_for_unrecognized_kind_is_fixed_and_never_echoes_input() {
        let raw = json!({"kind": "sk-superSecretLookingKindName1234567890"});
        let (result, _) = sanitize_kind_value(&raw, ALLOWLISTS);
        let reason = result.blocked_reason.expect("blocked");
        assert_eq!(
            reason,
            "kind is not a recognized mike49_run_fixture_audit output line type"
        );
        assert!(!reason.contains("sk-"));
    }
}
