//! Additional tests for `config_bridge/reader.rs` — split out to keep
//! `reader_tests.rs` under the 1000-line file-size ratchet.
//!
//! Included as `mod ext` inside `reader_tests.rs`, so `use super::*` gives
//! access to all helpers and types from that module.

use super::*;

// ── Numerics inheritance tests ────────────────────────────────────────────────
//
// max_output_tokens and context_limit gain persona/global tiers.

#[test]
fn numeric_context_limit_inherits_from_persona_env() {
    let record = test_record();
    let runtime = buzz_agent_runtime();
    let tiers = persona_env_tiers("BUZZ_AGENT_MAX_CONTEXT_TOKENS", "200000");

    let surface = read_config_surface(&record, Some(runtime), None, &tiers);

    let field = surface.normalized.context_limit.unwrap();
    assert_eq!(field.value.as_deref(), Some("200000"));
    assert_eq!(field.origin, ConfigOrigin::PersonaDefault);
}

#[test]
fn record_max_tokens_overrides_global_env_with_secondary() {
    let mut record = test_record();
    record.env_vars.insert(
        "BUZZ_AGENT_MAX_OUTPUT_TOKENS".to_string(),
        "8192".to_string(),
    );
    let runtime = buzz_agent_runtime();
    let tiers = global_env_tiers("BUZZ_AGENT_MAX_OUTPUT_TOKENS", "16384");

    let surface = read_config_surface(&record, Some(runtime), None, &tiers);

    let field = surface.normalized.max_output_tokens.unwrap();
    assert_eq!(field.value.as_deref(), Some("8192"));
    assert_eq!(field.origin, ConfigOrigin::BuzzExplicit);
    // Global value is the overridden secondary.
    assert_eq!(field.overridden_value.as_deref(), Some("16384"));
    assert_eq!(field.overridden_origin, Some(ConfigOrigin::GlobalDefault));
}

// ── Env-vs-structured collision tests (plan v3, Phase 2) ─────────────────────

/// Collision test 1: persona structured prompt + global env BUZZ_ACP_SYSTEM_PROMPT
/// → global env wins (env block sits entirely above structured).
#[test]
fn global_env_prompt_wins_over_persona_structured_prompt() {
    let record = test_record();
    let runtime = test_runtime();
    let tiers = InheritedConfigTiers {
        global_env: {
            let mut m = BTreeMap::new();
            m.insert(
                "BUZZ_ACP_SYSTEM_PROMPT".to_string(),
                "global-env-prompt".to_string(),
            );
            m
        },
        persona_prompt: Some("persona-structured-prompt".to_string()),
        ..Default::default()
    };

    let surface = read_config_surface(&record, Some(runtime), None, &tiers);

    let prompt = surface.normalized.system_prompt.unwrap();
    assert_eq!(prompt.value.as_deref(), Some("global-env-prompt"));
    assert_eq!(prompt.origin, ConfigOrigin::GlobalDefault);
}

/// Collision test 2: structured persona/record model + higher user-env value at
/// the runtime's model key → env value wins.
#[test]
fn persona_env_model_wins_over_persona_structured_model() {
    let record = test_record(); // no record.model
    let runtime = test_runtime(); // GOOSE_MODEL
    let tiers = InheritedConfigTiers {
        persona_env: {
            let mut m = BTreeMap::new();
            m.insert("GOOSE_MODEL".to_string(), "env-model".to_string());
            m
        },
        persona_model: Some("struct-persona-model".to_string()),
        ..Default::default()
    };

    let surface = read_config_surface(&record, Some(runtime), None, &tiers);

    let model = surface.normalized.model.unwrap();
    // persona env outranks persona struct because env candidates precede struct
    assert_eq!(model.value.as_deref(), Some("env-model"));
    assert_eq!(model.origin, ConfigOrigin::PersonaDefault);
}

/// Collision test 3: no env representation → structured persona/record/global
/// fallback and provenance remain intact.
#[test]
fn structured_fallback_intact_when_no_env_representation() {
    let record = test_record(); // no record.model, no env vars
    let runtime = test_runtime();
    let tiers = InheritedConfigTiers {
        persona_model: Some("struct-persona-model".to_string()),
        ..Default::default()
    };

    let surface = read_config_surface(&record, Some(runtime), None, &tiers);

    let model = surface.normalized.model.unwrap();
    assert_eq!(model.value.as_deref(), Some("struct-persona-model"));
    assert_eq!(model.origin, ConfigOrigin::PersonaDefault);
}

// ── Post-sanitization fallthrough test ───────────────────────────────────────
//
// Sanitization itself happens at the command boundary in `build_inherited_tiers`
// (a value with a NUL byte or an oversize value is dropped from the tier) and is
// pinned by the tests in `commands/agent_config_tests.rs`. The reader only ever
// sees the sanitized result, so what it must guarantee is the downstream half:
// a key stripped from one tier falls through to the next.

/// A key absent from the global env tier — the shape the reader sees after the
/// command boundary strips an invalid value — falls through to the persona tier.
#[test]
fn post_sanitization_empty_global_env_falls_through_to_persona_tier() {
    let record = test_record();
    let runtime = buzz_agent_rt();
    // No global env (stripped); persona provides the valid fallback.
    let tiers = persona_env_tiers("BUZZ_AGENT_THINKING_EFFORT", "medium");

    let surface = read_config_surface(&record, Some(runtime), None, &tiers);

    // Persona value surfaces instead of the stripped global value.
    let effort = surface.normalized.thinking_effort.unwrap();
    assert_eq!(effort.value.as_deref(), Some("medium"));
    assert_eq!(effort.origin, ConfigOrigin::PersonaDefault);
}

// ── Pass-3 prompt collision test ─────────────────────────────────────────────
//
// From Thufir's pass-3 verdict MINOR clarification (promoted to required):
// definition-less record with both structured and env prompt — env wins.

/// Pass-3 clarification: record.system_prompt = A + record env
/// BUZZ_ACP_SYSTEM_PROMPT = B → B wins as BuzzExplicit.
/// The env block sits above the struct block per v3 candidate-preparation
/// contract; current reader semantics (struct before env) would be wrong.
#[test]
fn record_env_prompt_wins_over_record_struct_prompt_as_buzz_explicit() {
    let mut record = test_record();
    record.system_prompt = Some("struct-prompt-A".to_string());
    record.env_vars.insert(
        "BUZZ_ACP_SYSTEM_PROMPT".to_string(),
        "env-prompt-B".to_string(),
    );
    let runtime = test_runtime();

    let surface = read_config_surface(&record, Some(runtime), None, &no_tiers());

    let prompt = surface.normalized.system_prompt.unwrap();
    assert_eq!(prompt.value.as_deref(), Some("env-prompt-B"));
    assert_eq!(prompt.origin, ConfigOrigin::BuzzExplicit);
    // Struct prompt is the secondary.
    assert_eq!(prompt.overridden_value.as_deref(), Some("struct-prompt-A"));
    assert_eq!(prompt.overridden_origin, Some(ConfigOrigin::BuzzExplicit));
}

// ── Definition env tier tests (Layer 2b) ─────────────────────────────────────
//
// The harness definition's `env` block sits below global env and above
// structured values in spawn's precedence (Layer 2b). These tests exercise
// the reader's mapping of that tier to `HarnessDefault` origin.

/// Definition env wins over structured persona model when no user-env or
/// global-env candidate is present.
#[test]
fn definition_env_beats_structured_persona_model() {
    let record = test_record(); // no record.model, no record.env_vars
    let runtime = test_runtime(); // model_env_var = "GOOSE_MODEL"
    let tiers = InheritedConfigTiers {
        definition_env: {
            let mut m = BTreeMap::new();
            m.insert("GOOSE_MODEL".to_string(), "harness-model".to_string());
            m
        },
        persona_model: Some("persona-struct-model".to_string()),
        ..Default::default()
    };

    let surface = read_config_surface(&record, Some(runtime), None, &tiers);

    let model = surface.normalized.model.unwrap();
    assert_eq!(model.value.as_deref(), Some("harness-model"));
    assert_eq!(model.origin, ConfigOrigin::HarnessDefault);
    // Structured persona model is the overridden secondary.
    assert_eq!(
        model.overridden_value.as_deref(),
        Some("persona-struct-model")
    );
    assert_eq!(model.overridden_origin, Some(ConfigOrigin::PersonaDefault));
}

/// Global env beats definition env — user-settable tiers always win over the
/// harness author's defaults.
#[test]
fn global_env_beats_definition_env() {
    let record = test_record();
    let runtime = test_runtime(); // model_env_var = "GOOSE_MODEL"
    let tiers = InheritedConfigTiers {
        global_env: {
            let mut m = BTreeMap::new();
            m.insert("GOOSE_MODEL".to_string(), "global-model".to_string());
            m
        },
        definition_env: {
            let mut m = BTreeMap::new();
            m.insert("GOOSE_MODEL".to_string(), "harness-model".to_string());
            m
        },
        ..Default::default()
    };

    let surface = read_config_surface(&record, Some(runtime), None, &tiers);

    let model = surface.normalized.model.unwrap();
    assert_eq!(model.value.as_deref(), Some("global-model"));
    assert_eq!(model.origin, ConfigOrigin::GlobalDefault);
    // Harness default is the overridden secondary.
    assert_eq!(model.overridden_value.as_deref(), Some("harness-model"));
    assert_eq!(model.overridden_origin, Some(ConfigOrigin::HarnessDefault));
}

/// A reserved key in the definition env is stripped by sanitization and must
/// not reach the reader. This test exercises the reader's contract (a key
/// absent from the tier falls through) — sanitization itself is pinned in
/// the `agent_config_tests.rs` constructor tests.
#[test]
fn reserved_key_absent_from_definition_env_falls_through() {
    let record = test_record();
    let runtime = test_runtime(); // model_env_var = "GOOSE_MODEL"
                                  // definition_env contains only an unrelated key — the env map here is what
                                  // the command boundary would produce after stripping a reserved key; the
                                  // reader must fall through to the next tier (persona structured model).
    let tiers = InheritedConfigTiers {
        definition_env: BTreeMap::new(), // stripped — nothing survives
        persona_model: Some("persona-struct-model".to_string()),
        ..Default::default()
    };

    let surface = read_config_surface(&record, Some(runtime), None, &tiers);

    let model = surface.normalized.model.unwrap();
    // Falls through to persona structured model.
    assert_eq!(model.value.as_deref(), Some("persona-struct-model"));
    assert_eq!(model.origin, ConfigOrigin::PersonaDefault);
}

// ── Phase 1: thought_level ACP category + alias normalization + B-collapse ───
//
// Plan v3 Phase 1: Live Goose effort is identified by ACP category
// `thought_level` (not the invented `effort` category). The matched entry's
// real `config_id` is used for AcpSetConfigOption write-back. All candidates
// are normalized before comparison so aliases compare equal to canonical
// forms (none↔off, xhigh↔max). B-collapse applies after normalization.

/// Goose real ACP shape: `category="thought_level"`, `id="thinking_effort"`,
/// canonical current value `high` → effort surfaces as `AcpConfigOption` with
/// write_via `AcpSetConfigOption { config_id: "thinking_effort" }`.
/// This pins that live Goose effort is actually read (the old `effort` category
/// would miss it entirely on a real Goose session).
#[test]
fn goose_real_acp_shape_thought_level_surfaces_and_routes_write_via_thinking_effort() {
    let record = test_record();
    let runtime = test_runtime(); // Goose with effort_normalization
    let cache = SessionConfigCache {
        config_options: vec![AcpConfigOptionEntry {
            config_id: "thinking_effort".to_string(),
            category: Some("thought_level".to_string()),
            display_name: Some("Thinking Effort".to_string()),
            current_value: Some("high".to_string()),
            options: vec![],
        }],
        available_modes: vec![],
        available_models: vec![],
        current_model: None,
        model_overridden: false,
        goose_native_config: None,
        captured_at: "".to_string(),
    };

    let surface = with_goose_path_root(Some("/nonexistent"), || {
        read_config_surface(&record, Some(runtime), Some(&cache), &no_tiers())
    });

    let effort = surface
        .normalized
        .thinking_effort
        .expect("Goose thought_level effort must surface");
    assert_eq!(effort.value.as_deref(), Some("high"));
    assert_eq!(effort.origin, ConfigOrigin::AcpConfigOption);
    assert!(
        matches!(
            &effort.write_via,
            ConfigWriteMechanism::AcpSetConfigOption { config_id }
                if config_id == "thinking_effort"
        ),
        "write_via must be AcpSetConfigOption with config_id=\"thinking_effort\", got {:?}",
        effort.write_via
    );
}

/// ★ Both ACP categories present → `thought_level` wins, `effort` is ignored.
/// Prevents hardcoding or first-match bugs that would pick the wrong entry.
#[test]
fn thought_level_category_wins_over_effort_category_when_both_present() {
    let record = test_record();
    let runtime = test_runtime(); // Goose
    let cache = SessionConfigCache {
        config_options: vec![
            // Legacy category comes first in the vec — must not win.
            AcpConfigOptionEntry {
                config_id: "effort".to_string(),
                category: Some("effort".to_string()),
                display_name: Some("Effort (legacy)".to_string()),
                current_value: Some("low".to_string()),
                options: vec![],
            },
            AcpConfigOptionEntry {
                config_id: "thinking_effort".to_string(),
                category: Some("thought_level".to_string()),
                display_name: Some("Thinking Effort".to_string()),
                current_value: Some("high".to_string()),
                options: vec![],
            },
        ],
        available_modes: vec![],
        available_models: vec![],
        current_model: None,
        model_overridden: false,
        goose_native_config: None,
        captured_at: "".to_string(),
    };

    let surface = with_goose_path_root(Some("/nonexistent"), || {
        read_config_surface(&record, Some(runtime), Some(&cache), &no_tiers())
    });

    let effort = surface
        .normalized
        .thinking_effort
        .expect("effort must surface from thought_level category");
    assert_eq!(
        effort.value.as_deref(),
        Some("high"),
        "thought_level value must win"
    );
    assert!(
        matches!(
            &effort.write_via,
            ConfigWriteMechanism::AcpSetConfigOption { config_id }
                if config_id == "thinking_effort"
        ),
        "write_via must use thought_level entry's config_id"
    );
}

/// B-collapse alias case — `none` and `off` are the same Goose effort after
/// normalization. ACP emits canonical `off`; record env has legacy `none`.
/// After normalization both equal `off` → B-collapse applies and the panel
/// shows the true baseline origin (BuzzExplicit), not AcpConfigOption.
/// write_via must still use AcpSetConfigOption (live session) even when
/// display origin falls through.
#[test]
fn b_collapse_none_and_off_are_equal_after_normalization() {
    let mut record = test_record();
    // Record env carries the alias `none` (legacy write).
    record
        .env_vars
        .insert("GOOSE_THINKING_EFFORT".to_string(), "none".to_string());
    let runtime = test_runtime(); // Goose with effort_normalization
                                  // Live ACP emits canonical `off`.
    let cache = SessionConfigCache {
        config_options: vec![AcpConfigOptionEntry {
            config_id: "thinking_effort".to_string(),
            category: Some("thought_level".to_string()),
            display_name: Some("Thinking Effort".to_string()),
            current_value: Some("off".to_string()),
            options: vec![],
        }],
        available_modes: vec![],
        available_models: vec![],
        current_model: None,
        model_overridden: false,
        goose_native_config: None,
        captured_at: "".to_string(),
    };

    let surface = with_goose_path_root(Some("/nonexistent"), || {
        read_config_surface(&record, Some(runtime), Some(&cache), &no_tiers())
    });

    let effort = surface
        .normalized
        .thinking_effort
        .expect("effort must surface");
    // Normalized `none` == canonical `off` — record env wins (above ACP in tier order).
    // Record env is above ACP, so it wins regardless of B-collapse — BuzzExplicit.
    assert_eq!(
        effort.value.as_deref(),
        Some("off"),
        "alias normalized to canonical"
    );
    assert_eq!(effort.origin, ConfigOrigin::BuzzExplicit);
}

/// B-collapse alias case — `xhigh` and `max` are the same Goose effort after
/// normalization. ACP emits canonical `max`; global env has alias `xhigh`.
/// After normalization both equal `max` → B-collapse: ACP falls through to
/// the non-ACP resolution, showing GlobalDefault as origin.
/// write_via stays AcpSetConfigOption (live session has an effort option).
#[test]
fn b_collapse_xhigh_and_max_are_equal_after_normalization() {
    let record = test_record();
    let runtime = test_runtime(); // Goose with effort_normalization
                                  // Global env has alias `xhigh`.
    let tiers = global_env_tiers("GOOSE_THINKING_EFFORT", "xhigh");
    // Live ACP emits canonical `max`.
    let cache = SessionConfigCache {
        config_options: vec![AcpConfigOptionEntry {
            config_id: "thinking_effort".to_string(),
            category: Some("thought_level".to_string()),
            display_name: Some("Thinking Effort".to_string()),
            current_value: Some("max".to_string()),
            options: vec![],
        }],
        available_modes: vec![],
        available_models: vec![],
        current_model: None,
        model_overridden: false,
        goose_native_config: None,
        captured_at: "".to_string(),
    };

    let surface = with_goose_path_root(Some("/nonexistent"), || {
        read_config_surface(&record, Some(runtime), Some(&cache), &tiers)
    });

    let effort = surface
        .normalized
        .thinking_effort
        .expect("effort must surface");
    // B-collapse: ACP `max` == normalized `xhigh` (`max`) → fall through to non-ACP.
    // Non-ACP resolution: global env `xhigh` normalizes to `max`, origin GlobalDefault.
    assert_eq!(
        effort.value.as_deref(),
        Some("max"),
        "alias normalized to canonical"
    );
    assert_eq!(
        effort.origin,
        ConfigOrigin::GlobalDefault,
        "B-collapse: equal-value ACP falls through to true baseline origin"
    );
    // write_via stays ACP-backed even when display provenance falls through.
    assert!(
        matches!(
            &effort.write_via,
            ConfigWriteMechanism::AcpSetConfigOption { config_id }
                if config_id == "thinking_effort"
        ),
        "equal-value collapse must retain ACP write_via with thinking_effort id"
    );
}

/// Alias normalization in record env: `GOOSE_THINKING_EFFORT=none` is
/// normalized to canonical `off` before being surfaced. The panel sees
/// the canonical form, never the raw alias.
#[test]
fn goose_record_env_alias_none_normalized_to_canonical_off() {
    let mut record = test_record();
    record
        .env_vars
        .insert("GOOSE_THINKING_EFFORT".to_string(), "none".to_string());
    let runtime = test_runtime();

    let surface = with_goose_path_root(Some("/nonexistent"), || {
        read_config_surface(&record, Some(runtime), None, &no_tiers())
    });

    let effort = surface
        .normalized
        .thinking_effort
        .expect("effort must surface");
    assert_eq!(
        effort.value.as_deref(),
        Some("off"),
        "none must normalize to off"
    );
    assert_eq!(effort.origin, ConfigOrigin::BuzzExplicit);
}

/// Legacy fallback: `category="effort"` is still honoured when no `thought_level`
/// entry exists (e.g. older adapters or test fixtures that predate the real category).
#[test]
fn legacy_effort_category_fallback_used_when_no_thought_level_present() {
    let record = test_record();
    let runtime = test_runtime(); // Goose
    let cache = SessionConfigCache {
        config_options: vec![AcpConfigOptionEntry {
            config_id: "effort_legacy".to_string(),
            category: Some("effort".to_string()),
            display_name: Some("Effort".to_string()),
            current_value: Some("medium".to_string()),
            options: vec![],
        }],
        available_modes: vec![],
        available_models: vec![],
        current_model: None,
        model_overridden: false,
        goose_native_config: None,
        captured_at: "".to_string(),
    };

    let surface = with_goose_path_root(Some("/nonexistent"), || {
        read_config_surface(&record, Some(runtime), Some(&cache), &no_tiers())
    });

    let effort = surface
        .normalized
        .thinking_effort
        .expect("legacy effort category must surface as fallback");
    assert_eq!(effort.value.as_deref(), Some("medium"));
    assert_eq!(effort.origin, ConfigOrigin::AcpConfigOption);
    // write_via uses the actual config_id from the matched entry.
    assert!(
        matches!(
            &effort.write_via,
            ConfigWriteMechanism::AcpSetConfigOption { config_id }
                if config_id == "effort_legacy"
        ),
        "fallback must retain entry's own config_id, not a hardcoded value"
    );
}

// ── ACP effort normalization tests ────────────────────────────────────────────
//
// Plan v3 Delta 1: every candidate — native, legacy, ACP, file — normalized
// before validity, precedence, override tracking, and B equality.
// ACP effort must be canonicalized through `effort_norm` before comparison;
// aliases (`none`→`off`, `xhigh`→`max`, case-fold) must collapse; invalid
// values (e.g. `minimal`) must be treated as absent so lower tiers win.

/// ACP `HIGH` (wrong case) against global `high` — must normalize to `high` and B-collapse.
#[test]
fn acp_effort_case_alias_collapses_against_global_canonical() {
    let record = test_record();
    let runtime = test_runtime(); // Goose with effort_normalization
    let cache = SessionConfigCache {
        config_options: vec![AcpConfigOptionEntry {
            config_id: "thinking_effort".to_string(),
            category: Some("thought_level".to_string()),
            display_name: Some("Thinking Effort".to_string()),
            current_value: Some("HIGH".to_string()), // wrong case
            options: vec![],
        }],
        available_modes: vec![],
        available_models: vec![],
        current_model: None,
        model_overridden: false,
        goose_native_config: None,
        captured_at: "".to_string(),
    };

    // Global env has canonical `high`.
    let mut tiers = no_tiers();
    tiers
        .global_env
        .insert("GOOSE_THINKING_EFFORT".to_string(), "high".to_string());

    let surface = with_goose_path_root(Some("/nonexistent"), || {
        read_config_surface(&record, Some(runtime), Some(&cache), &tiers)
    });

    let effort = surface
        .normalized
        .thinking_effort
        .expect("effort must surface");
    // ACP `HIGH` normalizes to `high` == global `high` → B-collapse: falls through to
    // non-ACP resolution, origin is GlobalDefault (not AcpConfigOption).
    assert_eq!(
        effort.value.as_deref(),
        Some("high"),
        "case-folded ACP value must collapse to canonical"
    );
    assert_eq!(
        effort.origin,
        ConfigOrigin::GlobalDefault,
        "B-collapse must report GlobalDefault, not AcpConfigOption"
    );
}

/// ACP `none` against global `off` — alias-equal after normalization, must B-collapse.
#[test]
fn acp_effort_alias_none_collapses_against_global_off() {
    let record = test_record();
    let runtime = test_runtime();
    let cache = SessionConfigCache {
        config_options: vec![AcpConfigOptionEntry {
            config_id: "thinking_effort".to_string(),
            category: Some("thought_level".to_string()),
            display_name: Some("Thinking Effort".to_string()),
            current_value: Some("none".to_string()), // alias for `off`
            options: vec![],
        }],
        available_modes: vec![],
        available_models: vec![],
        current_model: None,
        model_overridden: false,
        goose_native_config: None,
        captured_at: "".to_string(),
    };

    let mut tiers = no_tiers();
    tiers
        .global_env
        .insert("GOOSE_THINKING_EFFORT".to_string(), "off".to_string());

    let surface = with_goose_path_root(Some("/nonexistent"), || {
        read_config_surface(&record, Some(runtime), Some(&cache), &tiers)
    });

    let effort = surface
        .normalized
        .thinking_effort
        .expect("effort must surface");
    assert_eq!(
        effort.value.as_deref(),
        Some("off"),
        "ACP `none` must normalize to `off` and B-collapse with global `off`"
    );
    assert_eq!(effort.origin, ConfigOrigin::GlobalDefault);
}

/// ACP `xhigh` against persona `max` — alias-equal after normalization, must B-collapse.
#[test]
fn acp_effort_alias_xhigh_collapses_against_persona_max() {
    let record = test_record();
    let runtime = test_runtime();
    let cache = SessionConfigCache {
        config_options: vec![AcpConfigOptionEntry {
            config_id: "thinking_effort".to_string(),
            category: Some("thought_level".to_string()),
            display_name: Some("Thinking Effort".to_string()),
            current_value: Some("xhigh".to_string()), // alias for `max`
            options: vec![],
        }],
        available_modes: vec![],
        available_models: vec![],
        current_model: None,
        model_overridden: false,
        goose_native_config: None,
        captured_at: "".to_string(),
    };

    let mut tiers = no_tiers();
    tiers
        .persona_env
        .insert("GOOSE_THINKING_EFFORT".to_string(), "max".to_string());

    let surface = with_goose_path_root(Some("/nonexistent"), || {
        read_config_surface(&record, Some(runtime), Some(&cache), &tiers)
    });

    let effort = surface
        .normalized
        .thinking_effort
        .expect("effort must surface");
    assert_eq!(
        effort.value.as_deref(),
        Some("max"),
        "ACP `xhigh` must normalize to `max` and B-collapse with persona `max`"
    );
    assert_eq!(effort.origin, ConfigOrigin::PersonaDefault);
}

/// ACP value invalid for Goose (`minimal`) — must be skipped; lower valid candidate wins.
#[test]
fn acp_effort_invalid_for_runtime_skipped_lower_tier_wins() {
    let record = test_record();
    let runtime = test_runtime();
    let cache = SessionConfigCache {
        config_options: vec![AcpConfigOptionEntry {
            config_id: "thinking_effort".to_string(),
            category: Some("thought_level".to_string()),
            display_name: Some("Thinking Effort".to_string()),
            current_value: Some("minimal".to_string()), // Goose does not accept "minimal"
            options: vec![],
        }],
        available_modes: vec![],
        available_models: vec![],
        current_model: None,
        model_overridden: false,
        goose_native_config: None,
        captured_at: "".to_string(),
    };

    // Global env has a valid Goose effort — should win because ACP `minimal` is skipped.
    let mut tiers = no_tiers();
    tiers
        .global_env
        .insert("GOOSE_THINKING_EFFORT".to_string(), "medium".to_string());

    let surface = with_goose_path_root(Some("/nonexistent"), || {
        read_config_surface(&record, Some(runtime), Some(&cache), &tiers)
    });

    let effort = surface
        .normalized
        .thinking_effort
        .expect("effort must surface from global tier");
    assert_eq!(
        effort.value.as_deref(),
        Some("medium"),
        "invalid ACP `minimal` must be skipped; global `medium` must win"
    );
    assert_eq!(
        effort.origin,
        ConfigOrigin::GlobalDefault,
        "origin must be GlobalDefault after ACP skip"
    );
    // write_via stays AcpSetConfigOption — the route targets the live option,
    // not its current value; an invalid live value changes display resolution
    // but not writability (plan v3 Phase 1 ruling).
    assert!(
        matches!(
            &effort.write_via,
            ConfigWriteMechanism::AcpSetConfigOption { config_id }
                if config_id == "thinking_effort"
        ),
        "write_via must remain AcpSetConfigOption{{thinking_effort}} even when ACP value is invalid; got {:?}",
        effort.write_via
    );
}

// ── Definition-tier effort alias policy (reader) ──────────────────────────────
//
// plan v3: legacy alias (`BUZZ_AGENT_THINKING_EFFORT`) is consumed only at
// record and persona tiers; definition and global tiers use native-key-only
// lookup. These tests verify the reader enforces the same tier boundary as
// the spawn bridge.

/// Definition env with legacy key `BUZZ_AGENT_THINKING_EFFORT=high` for Goose
/// → reader must NOT surface it as effort (definition tier excludes legacy alias).
#[test]
fn definition_legacy_key_excluded_reader() {
    let record = test_record();
    let runtime = test_runtime(); // Goose
    let mut tiers = no_tiers();
    tiers
        .definition_env
        .insert("BUZZ_AGENT_THINKING_EFFORT".to_string(), "high".to_string());

    let surface = with_goose_path_root(Some("/nonexistent"), || {
        read_config_surface(&record, Some(runtime), None, &tiers)
    });

    assert!(
        surface.normalized.thinking_effort.is_none(),
        "definition-tier legacy key must NOT be surfaced as effort for Goose"
    );
}

/// Definition env with native key `GOOSE_THINKING_EFFORT=medium` for Goose
/// → reader surfaces it correctly (native key at definition tier is accepted).
#[test]
fn definition_native_key_accepted_reader() {
    let record = test_record();
    let runtime = test_runtime(); // Goose
    let mut tiers = no_tiers();
    tiers
        .definition_env
        .insert("GOOSE_THINKING_EFFORT".to_string(), "medium".to_string());

    let surface = with_goose_path_root(Some("/nonexistent"), || {
        read_config_surface(&record, Some(runtime), None, &tiers)
    });

    let effort = surface
        .normalized
        .thinking_effort
        .expect("definition native key must surface as effort");
    assert_eq!(effort.value.as_deref(), Some("medium"));
}
