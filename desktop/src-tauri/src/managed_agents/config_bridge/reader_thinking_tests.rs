//! Thinking-effort tests for `config_bridge/reader.rs` — the live-ACP
//! `thought_level` tier and its env-var fallbacks. Split out of
//! `reader_tests.rs` to respect the repo's file-size ratchet.
//!
//! Included as `mod thinking` inside `reader_tests.rs`, so `use super::*`
//! gives access to all helpers and types from that module.

use super::*;

/// AC-5 (conflicting-ACP): inherited effort set (global=high) + live ACP effort=low
/// → ACP wins as primary (AcpConfigOption), global is the overridden secondary.
#[test]
fn acp_effort_wins_over_inherited_global_effort_as_secondary() {
    let record = test_record();
    let runtime = buzz_agent_rt();
    let cache = SessionConfigCache {
        config_options: vec![AcpConfigOptionEntry {
            config_id: "effort".to_string(),
            category: Some("thought_level".to_string()),
            display_name: Some("Effort".to_string()),
            current_value: Some("low".to_string()),
            options: vec![],
        }],
        available_modes: vec![],
        available_models: vec![],
        current_model: None,
        model_overridden: false,
        goose_native_config: None,
        captured_at: "".to_string(),
    };
    let tiers = global_env_tiers("BUZZ_AGENT_THINKING_EFFORT", "high");

    let surface = read_config_surface(&record, Some(runtime), Some(&cache), &tiers);

    let effort = surface
        .normalized
        .thinking_effort
        .expect("effort must surface from ACP tier");
    // Live ACP value wins.
    assert_eq!(effort.value.as_deref(), Some("low"));
    assert_eq!(effort.origin, ConfigOrigin::AcpConfigOption);
    // Global is surfaced as the overridden baseline.
    assert_eq!(effort.overridden_value.as_deref(), Some("high"));
    assert_eq!(effort.overridden_origin, Some(ConfigOrigin::GlobalDefault));
    assert!(matches!(
        effort.write_via,
        ConfigWriteMechanism::AcpSetConfigOption { ref config_id } if config_id == "effort"
    ));
}

#[test]
fn codex_thought_level_uses_advertised_reasoning_effort_config_id() {
    let record = test_record();
    let runtime = crate::managed_agents::known_acp_runtime_exact("codex").unwrap();
    let cache = SessionConfigCache {
        config_options: vec![AcpConfigOptionEntry {
            config_id: "reasoning_effort".to_string(),
            category: Some("thought_level".to_string()),
            display_name: Some("Reasoning effort".to_string()),
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

    let surface = read_config_surface(&record, Some(runtime), Some(&cache), &no_tiers());
    let effort = surface.normalized.thinking_effort.unwrap();
    assert_eq!(effort.value.as_deref(), Some("high"));
    assert_eq!(effort.origin, ConfigOrigin::AcpConfigOption);
    assert!(matches!(
        effort.write_via,
        ConfigWriteMechanism::AcpSetConfigOption { ref config_id }
            if config_id == "reasoning_effort"
    ));
}

#[test]
fn pre_spawn_effort_uses_direct_env_fallback() {
    let mut record = test_record();
    record
        .env_vars
        .insert("GOOSE_THINKING_EFFORT".to_string(), "medium".to_string());

    let effort = read_config_surface(&record, Some(test_runtime()), None, &no_tiers())
        .normalized
        .thinking_effort
        .unwrap();
    assert_eq!(effort.value.as_deref(), Some("medium"));
    assert!(matches!(
        effort.write_via,
        ConfigWriteMechanism::RespawnWithEnvVar { ref env_key }
            if env_key == "GOOSE_THINKING_EFFORT"
    ));
}
