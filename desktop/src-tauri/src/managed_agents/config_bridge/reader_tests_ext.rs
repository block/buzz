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

    let surface = read_config_surface(&record, Some(runtime), None, &tiers, None);

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

    let surface = read_config_surface(&record, Some(runtime), None, &tiers, None);

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

    let surface = read_config_surface(&record, Some(runtime), None, &tiers, None);

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

    let surface = read_config_surface(&record, Some(runtime), None, &tiers, None);

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

    let surface = read_config_surface(&record, Some(runtime), None, &tiers, None);

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

    let surface = read_config_surface(&record, Some(runtime), None, &tiers, None);

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

    let surface = read_config_surface(&record, Some(runtime), None, &no_tiers(), None);

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

    let surface = read_config_surface(&record, Some(runtime), None, &tiers, None);

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

    let surface = read_config_surface(&record, Some(runtime), None, &tiers, None);

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

    let surface = read_config_surface(&record, Some(runtime), None, &tiers, None);

    let model = surface.normalized.model.unwrap();
    // Falls through to persona structured model.
    assert_eq!(model.value.as_deref(), Some("persona-struct-model"));
    assert_eq!(model.origin, ConfigOrigin::PersonaDefault);
}

// ── B4/B5 canonical effort_level tier tests ────────────────────────────────
//
// record.effort_level is the Buzz-canonical seeded value (the effort a spawn
// applies at next session start via `apply_effort_env`). It must surface as
// BuzzExplicit and take precedence over the config-file tier, but not over a
// record env var override.

/// B4: record.effort_level surfaces as BuzzExplicit when no env var is set.
#[test]
fn b4_canonical_effort_level_surfaces_as_buzz_explicit() {
    let mut record = test_record();
    record.effort_level = Some("high".to_string());
    let runtime = buzz_agent_runtime();
    let surface = read_config_surface(&record, Some(runtime), None, &no_tiers(), None);
    let effort = surface
        .normalized
        .thinking_effort
        .expect("effort must surface from canonical record tier");
    assert_eq!(effort.value.as_deref(), Some("high"));
    assert_eq!(effort.origin, ConfigOrigin::BuzzExplicit);
}

/// B4: record.effort_level shadows the config-file tier.
#[test]
fn b4_canonical_effort_level_shadows_file_tier() {
    let mut record = test_record();
    record.effort_level = Some("medium".to_string());
    // No env var set — the config-file tier would win if canonical were absent.
    let runtime = buzz_agent_runtime();
    let surface = read_config_surface(&record, Some(runtime), None, &no_tiers(), None);
    let effort = surface
        .normalized
        .thinking_effort
        .expect("canonical effort must shadow file tier");
    assert_eq!(effort.value.as_deref(), Some("medium"));
    assert_eq!(effort.origin, ConfigOrigin::BuzzExplicit);
}

/// B4: a record env var override still wins over record.effort_level, which
/// becomes the overridden baseline.
#[test]
fn b4_record_env_var_wins_over_canonical_effort_level() {
    let mut record = test_record();
    record.effort_level = Some("low".to_string());
    record
        .env_vars
        .insert("BUZZ_AGENT_THINKING_EFFORT".to_string(), "high".to_string());
    let runtime = buzz_agent_runtime();
    let surface = read_config_surface(&record, Some(runtime), None, &no_tiers(), None);
    let effort = surface
        .normalized
        .thinking_effort
        .expect("env var must win over canonical effort");
    assert_eq!(effort.value.as_deref(), Some("high"));
    assert_eq!(effort.origin, ConfigOrigin::BuzzExplicit);
    assert_eq!(effort.overridden_value.as_deref(), Some("low"));
}

/// B4: None effort_level does not introduce a spurious tier.
#[test]
fn b4_none_canonical_effort_does_not_surface() {
    let record = test_record(); // effort_level defaults to None
    let runtime = buzz_agent_runtime();
    let surface = read_config_surface(&record, Some(runtime), None, &no_tiers(), None);
    assert!(
        surface.normalized.thinking_effort.is_none(),
        "effort field must be absent when no tier has a value"
    );
}

// ── CLAUDE_CONFIG_DIR path resolution (#3493) ─────────────────────────────────

#[test]
fn claude_mcp_config_path_honors_custom_claude_config_dir() {
    // #3493: mcp_config_file_path_for_runtime must use the custom dir when
    // claude_config_dir is Some, not fall back to ~/.claude.json.
    let record = test_record();
    let runtime = &KnownAcpRuntime {
        id: "claude",
        config_file_path: Some("~/.claude/settings.json"),
        ..*test_runtime()
    };
    let custom_dir = std::path::PathBuf::from("/custom/config/dir");
    let surface = read_config_surface(&record, Some(runtime), None, &no_tiers(), Some(&custom_dir));

    let mcp_path = surface
        .sources
        .mcp_config_file_path
        .expect("mcp_config_file_path must be present for claude runtime");
    assert_eq!(
        std::path::Path::new(&mcp_path),
        custom_dir.join(".claude.json"),
        "mcp config path must be <custom_dir>/.claude.json when CLAUDE_CONFIG_DIR is set"
    );
    assert!(
        surface.claude_config_dir_custom,
        "claude_config_dir_custom must be true when a custom dir was passed"
    );
}

#[test]
fn claude_config_dir_none_falls_back_to_home_claude_json() {
    // #3493: None (i.e. the caller stripped an empty string) must resolve to
    // the default ~/.claude.json path, matching Claude's `CLAUDE_CONFIG_DIR || homedir()`.
    let record = test_record();
    let runtime = &KnownAcpRuntime {
        id: "claude",
        config_file_path: Some("~/.claude/settings.json"),
        ..*test_runtime()
    };
    let surface = read_config_surface(&record, Some(runtime), None, &no_tiers(), None);
    assert!(
        !surface.claude_config_dir_custom,
        "claude_config_dir_custom must be false when dir is None (unset)"
    );
    assert!(
        surface
            .sources
            .mcp_config_file_path
            .as_deref()
            .is_some_and(|p| p.ends_with(".claude.json")),
        "mcp path must fall back to ~/.claude.json when no custom dir"
    );
}

/// F1 regression: the effort control is selected by its `thought_level` category,
/// and the running value, the write config id, and the picker options all derive
/// from that single entry — even when the adapter's config id is a nonliteral
/// value and differs from the canonical (configured) effort.
///
/// Live shape: `id="thinking-level", category="thought_level", currentValue="default"`
/// while canonical `record.effort_level=high`. Both facts must render: configured
/// `high` as the value and running `default` as the overridden secondary; the
/// write mechanism must carry the adapter's real id, never a hardcoded `"effort"`.
#[test]
fn effort_option_selected_by_category_drives_all_facts() {
    let mut record = test_record();
    record.effort_level = Some("high".to_string());
    let runtime = buzz_agent_rt();
    let cache = SessionConfigCache {
        config_options: vec![AcpConfigOptionEntry {
            config_id: "thinking-level".to_string(),
            category: Some("thought_level".to_string()),
            display_name: Some("Thinking level".to_string()),
            current_value: Some("default".to_string()),
            options: vec![
                AcpConfigOptionValue {
                    value: "default".to_string(),
                    display_name: Some("Default".to_string()),
                },
                AcpConfigOptionValue {
                    value: "high".to_string(),
                    display_name: Some("High".to_string()),
                },
            ],
        }],
        available_modes: vec![],
        available_models: vec![],
        current_model: None,
        model_overridden: false,
        goose_native_config: None,
        captured_at: "".to_string(),
    };
    let tiers = InheritedConfigTiers::default();

    let surface = read_config_surface(&record, Some(runtime), Some(&cache), &tiers, None);

    // Two-facts display: configured `high` wins, running `default` is the secondary.
    let effort = surface
        .normalized
        .thinking_effort
        .expect("effort must surface with both configured and running facts");
    assert_eq!(effort.value.as_deref(), Some("high"));
    assert_eq!(effort.origin, ConfigOrigin::BuzzExplicit);
    assert_eq!(effort.overridden_value.as_deref(), Some("default"));
    assert_eq!(
        effort.overridden_origin,
        Some(ConfigOrigin::AcpConfigOption)
    );

    // Write mechanism carries the adapter's real id, never a hardcoded "effort".
    match &effort.write_via {
        ConfigWriteMechanism::AcpSetConfigOption { config_id } => {
            assert_eq!(config_id, "thinking-level");
        }
        other => panic!("expected AcpSetConfigOption with adapter id, got {other:?}"),
    }

    // Picker metadata derives from the same entry.
    assert_eq!(surface.effort_config_id.as_deref(), Some("thinking-level"));
    assert_eq!(
        surface
            .effort_options
            .iter()
            .map(|o| o.value.as_str())
            .collect::<Vec<_>>(),
        vec!["default", "high"],
    );
}

// ── #3493: config_file_path follows a custom CLAUDE_CONFIG_DIR ─────────────────

#[test]
fn claude_custom_config_dir_reports_isolated_settings_path() {
    let record = test_record();
    let runtime = &KnownAcpRuntime {
        id: "claude",
        config_file_path: Some("~/.claude/settings.json"),
        ..*test_runtime()
    };
    let custom = std::path::Path::new("/tmp/iso-config");

    let surface = read_config_surface(&record, Some(runtime), None, &no_tiers(), Some(custom));

    // The reported settings path is rooted at the custom dir the reader used,
    // not the static ~/.claude/settings.json metadata. Compare as paths so the
    // separator is native (Windows joins with `\`, not `/`).
    assert_eq!(
        surface
            .sources
            .config_file_path
            .as_deref()
            .map(std::path::Path::new),
        Some(custom.join("settings.json").as_path()),
    );
    // And the MCP file attribution follows the same custom root.
    assert_eq!(
        surface
            .sources
            .mcp_config_file_path
            .as_deref()
            .map(std::path::Path::new),
        Some(custom.join(".claude.json").as_path()),
    );
}

#[test]
fn claude_default_config_dir_reports_static_settings_path() {
    let record = test_record();
    let runtime = &KnownAcpRuntime {
        id: "claude",
        config_file_path: Some("~/.claude/settings.json"),
        ..*test_runtime()
    };

    let surface = read_config_surface(&record, Some(runtime), None, &no_tiers(), None);

    // With no custom dir, the settings path resolves the static tilde metadata.
    // Compare the trailing components as a path so the check is separator-native.
    assert!(surface
        .sources
        .config_file_path
        .as_deref()
        .map(std::path::Path::new)
        .is_some_and(|p| p.ends_with(".claude/settings.json")));
    assert!(surface
        .sources
        .config_file_path
        .as_deref()
        .is_some_and(|p| !p.starts_with('~')));
}
