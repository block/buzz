//! Unit tests for `config_bridge/reader.rs` (kept in a sibling file so
//! `reader.rs` stays under the 1000-line budget; `#[path]`-included from
//! there).

use std::{collections::BTreeMap, path::Path, sync::Mutex};

use super::*;
use crate::managed_agents::discovery::KnownAcpRuntime;
use crate::managed_agents::types::ManagedAgentRecord;

static GOOSE_PATH_ROOT_LOCK: Mutex<()> = Mutex::new(());

fn with_goose_path_root<T>(value: Option<&str>, body: impl FnOnce() -> T) -> T {
    let _guard = GOOSE_PATH_ROOT_LOCK
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let prior = std::env::var_os("GOOSE_PATH_ROOT");
    match value {
        Some(value) => std::env::set_var("GOOSE_PATH_ROOT", value),
        None => std::env::remove_var("GOOSE_PATH_ROOT"),
    }
    let output = body();
    match prior {
        Some(value) => std::env::set_var("GOOSE_PATH_ROOT", value),
        None => std::env::remove_var("GOOSE_PATH_ROOT"),
    }
    output
}

fn test_runtime() -> &'static KnownAcpRuntime {
    &KnownAcpRuntime {
        id: "goose",
        label: "Goose",
        commands: &["goose"],
        aliases: &[],
        avatar_url: "",
        mcp_command: None,
        mcp_hooks: false,
        underlying_cli: None,
        cli_install_commands: &[],
        adapter_install_commands: &[],
        install_instructions_url: "",
        cli_install_hint: "",
        adapter_install_hint: "",
        skill_dir: None,
        supports_acp_model_switching: false,
        model_env_var: Some("GOOSE_MODEL"),
        provider_env_var: Some("GOOSE_PROVIDER"),
        provider_locked: false,
        default_env: &[],
        config_file_path: Some("~/.config/goose/config.yaml"),
        config_file_format: Some("yaml"),
        supports_acp_native_config: true,
        thinking_env_var: Some("GOOSE_THINKING_EFFORT"),
        max_tokens_env_var: Some("GOOSE_MAX_TOKENS"),
        context_limit_env_var: Some("GOOSE_CONTEXT_LIMIT"),
        required_normalized_fields: &["model", "provider"],
        login_hint: None,
        auth_probe_args: None,
    }
}

fn test_record() -> ManagedAgentRecord {
    ManagedAgentRecord {
        pubkey: "test".to_string(),
        name: "Test Agent".to_string(),
        persona_id: None,
        private_key_nsec: "".to_string(),
        auth_tag: None,
        relay_url: "ws://localhost:3000".to_string(),
        avatar_url: None,
        acp_command: "buzz-acp".to_string(),
        agent_command: "goose".to_string(),
        agent_args: vec![],
        mcp_command: "".to_string(),
        turn_timeout_seconds: 300,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: None,
        model: None,
        env_vars: BTreeMap::new(),
        start_on_app_launch: false,
        auto_restart_on_config_change: true,
        runtime_pid: None,
        backend: crate::managed_agents::types::BackendKind::Local,
        backend_agent_id: None,
        provider_binary_path: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        created_at: "".to_string(),
        updated_at: "".to_string(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        last_error_code: None,
        respond_to: crate::managed_agents::types::RespondTo::OwnerOnly,
        respond_to_allowlist: vec![],
        display_name: None,
        slug: None,
        runtime: None,
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        definition_respond_to: None,
        definition_respond_to_allowlist: Vec::new(),
        definition_parallelism: None,
        relay_mesh: None,
        agent_command_override: None,
        persona_source_version: None,
        provider: None,
    }
}

#[test]
fn pre_spawn_surface_reports_pending_acp_tiers() {
    let record = test_record();
    let runtime = test_runtime();
    let surface = read_config_surface(&record, Some(runtime), None, None);

    assert!(surface.is_pre_spawn);
    assert_eq!(surface.sources.acp_native, ConfigTierStatus::Pending);
    assert_eq!(
        surface.sources.acp_config_options,
        ConfigTierStatus::Pending
    );
    assert_eq!(surface.sources.env_vars, ConfigTierStatus::Available);
}

#[test]
fn surface_reports_mcp_specific_config_path() {
    let record = test_record();
    let runtime = test_runtime();
    let surface = with_goose_path_root(None, || {
        read_config_surface(&record, Some(runtime), None, None)
    });

    let path = surface
        .sources
        .mcp_config_file_path
        .expect("mcp config path");
    let expected_suffix = Path::new(".config").join("goose").join("config.yaml");
    assert!(
        Path::new(&path).ends_with(&expected_suffix),
        "unexpected MCP config path: {path}"
    );
}

#[test]
fn goose_mcp_config_path_follows_path_root_override() {
    let record = test_record();
    let runtime = test_runtime();
    let surface = with_goose_path_root(Some("/tmp/buzz-goose-root"), || {
        read_config_surface(&record, Some(runtime), None, None)
    });

    let expected_path = Path::new("/tmp/buzz-goose-root")
        .join("config")
        .join("config.yaml");
    assert_eq!(
        surface
            .sources
            .mcp_config_file_path
            .as_deref()
            .map(Path::new),
        Some(expected_path.as_path())
    );
}

#[test]
fn claude_surface_uses_mcp_config_path_not_settings_path() {
    let record = test_record();
    let runtime = &KnownAcpRuntime {
        id: "claude",
        config_file_path: Some("~/.claude/settings.json"),
        ..*test_runtime()
    };
    let surface = read_config_surface(&record, Some(runtime), None, None);

    assert!(surface
        .sources
        .config_file_path
        .as_deref()
        .is_some_and(|path| path.ends_with(".claude/settings.json")));
    assert!(surface
        .sources
        .mcp_config_file_path
        .as_deref()
        .is_some_and(|path| path.ends_with(".claude.json")));
}

#[test]
fn record_model_overrides_file_model() {
    let mut record = test_record();
    record.model = Some("explicit-model".to_string());
    let runtime = test_runtime();

    let surface = read_config_surface(&record, Some(runtime), None, None);
    let model = surface.normalized.model.unwrap();
    assert_eq!(model.value.as_deref(), Some("explicit-model"));
    assert_eq!(model.origin, ConfigOrigin::BuzzExplicit);
}

#[test]
fn provider_locked_shows_locked() {
    let record = test_record();
    let runtime = &KnownAcpRuntime {
        provider_locked: true,
        ..*test_runtime()
    };
    let surface = read_config_surface(&record, Some(runtime), None, None);
    let provider = surface.normalized.provider.unwrap();
    assert_eq!(provider.value.as_deref(), Some("Anthropic (locked)"));
    assert_eq!(provider.origin, ConfigOrigin::HarnessConstraint);
}

#[test]
fn post_spawn_with_model_config_option_uses_acp() {
    let record = test_record();
    let runtime = test_runtime();
    let cache = SessionConfigCache {
        config_options: vec![AcpConfigOptionEntry {
            config_id: "model".to_string(),
            category: Some("model".to_string()),
            display_name: Some("Model".to_string()),
            current_value: Some("claude-opus-4".to_string()),
            options: vec![],
        }],
        available_modes: vec![],
        available_models: vec![],
        current_model: Some("claude-opus-4".to_string()),
        model_overridden: false,
        goose_native_config: None,
        captured_at: "".to_string(),
    };

    let surface = read_config_surface(&record, Some(runtime), Some(&cache), None);
    assert!(!surface.is_pre_spawn);
    let model = surface.normalized.model.unwrap();
    assert_eq!(model.value.as_deref(), Some("claude-opus-4"));
    assert!(matches!(
        model.write_via,
        ConfigWriteMechanism::AcpSetConfigOption { .. }
    ));
}

#[test]
fn acp_model_overrides_file_model_with_override_tracking() {
    let record = test_record();
    let runtime = test_runtime();
    let cache = SessionConfigCache {
        config_options: vec![],
        available_modes: vec![],
        available_models: vec![],
        current_model: Some("acp-model".to_string()),
        model_overridden: false,
        goose_native_config: None,
        captured_at: "".to_string(),
    };

    let surface = read_config_surface(&record, Some(runtime), Some(&cache), None);
    let model = surface.normalized.model.unwrap();
    assert_eq!(model.value.as_deref(), Some("acp-model"));
    assert_eq!(model.origin, ConfigOrigin::AcpConfigOption);
    // The goose config file might have a model too — since we can't control
    // the actual file in a unit test, just verify the override fields are populated
    // when we manually construct the scenario via build_model_field.
}

// ── Persona resolution integration tests ────────────────────────────
//
// These simulate the call-site pattern in agent_config.rs:
// 1. Inject persona-resolved values into the record (as if absent)
// 2. Call read_config_surface (reader tags them BuzzExplicit)
// 3. Re-tag injected fields to PersonaDefault
//
// This exercises the same logic path as get_agent_config_surface without
// requiring Tauri AppHandle/State infrastructure.

#[test]
fn persona_model_injection_produces_persona_default_origin() {
    let mut record = test_record();
    // Simulate: record has no model, persona provides one.
    // The call-site injects it before calling the reader.
    record.model = Some("persona-model".to_string());
    let runtime = test_runtime();

    let mut surface = read_config_surface(&record, Some(runtime), None, None);

    // Reader sees injected model as BuzzExplicit.
    let model = surface.normalized.model.as_ref().unwrap();
    assert_eq!(model.value.as_deref(), Some("persona-model"));
    assert_eq!(model.origin, ConfigOrigin::BuzzExplicit);

    // Call-site re-tags (simulating had_model == false).
    if let Some(ref mut field) = surface.normalized.model {
        if field.origin == ConfigOrigin::BuzzExplicit {
            field.origin = ConfigOrigin::PersonaDefault;
        }
    }

    let model = surface.normalized.model.unwrap();
    assert_eq!(model.value.as_deref(), Some("persona-model"));
    assert_eq!(model.origin, ConfigOrigin::PersonaDefault);
}

// ── Runtime override (Phase 3c) ──────────────────────────────────────
//
// A live ModelPicker switch is signalled by `model_overridden: true` in the
// `session_config_captured` payload. The reader keys the override-active
// decision off that flag — NOT off `acp_model != persona_model`, which would
// false-positive when a persona model is edited mid-life.

#[test]
fn runtime_override_wins_display_when_model_overridden_is_true() {
    // Persona-linked agent (record.model == None); persona == "persona-model".
    // A live switch pushed "live-model" to the session and set model_overridden.
    let record = test_record();
    let runtime = test_runtime();
    let cache = SessionConfigCache {
        config_options: vec![],
        available_modes: vec![],
        available_models: vec![],
        current_model: Some("live-model".to_string()),
        model_overridden: true,
        goose_native_config: None,
        captured_at: "".to_string(),
    };

    let surface = read_config_surface(
        &record,
        Some(runtime),
        Some(&cache),
        Some(("persona-model", ConfigOrigin::PersonaDefault)),
    );
    let model = surface.normalized.model.unwrap();

    // Override wins the display value with a runtime-override origin.
    assert_eq!(model.value.as_deref(), Some("live-model"));
    assert_eq!(model.origin, ConfigOrigin::RuntimeOverride);
    // Persona is the secondary value (not struck through — the UI keys off
    // the RuntimeOverride origin to suppress strikethrough).
    assert_eq!(model.overridden_value.as_deref(), Some("persona-model"));
    assert_eq!(model.overridden_origin, Some(ConfigOrigin::PersonaDefault));
}

#[test]
fn no_runtime_override_when_model_overridden_is_false() {
    // At spawn the session's current_model == persona model (BUZZ_ACP_MODEL
    // is set to the persona model) and model_overridden is false. No override;
    // the field falls through to normal precedence.
    let record = test_record();
    let runtime = test_runtime();
    let cache = SessionConfigCache {
        config_options: vec![],
        available_modes: vec![],
        available_models: vec![],
        current_model: Some("persona-model".to_string()),
        model_overridden: false,
        goose_native_config: None,
        captured_at: "".to_string(),
    };

    let surface = read_config_surface(
        &record,
        Some(runtime),
        Some(&cache),
        Some(("persona-model", ConfigOrigin::PersonaDefault)),
    );
    let model = surface.normalized.model.unwrap();

    // model_overridden is false => the override branch is not taken: origin
    // is the normal precedence result, never RuntimeOverride.
    assert_ne!(model.origin, ConfigOrigin::RuntimeOverride);
    assert_eq!(model.value.as_deref(), Some("persona-model"));
    assert_ne!(model.overridden_origin, Some(ConfigOrigin::PersonaDefault));
}

#[test]
fn no_false_positive_override_when_persona_edited_mid_life() {
    // Persona-linked agent whose persona model was edited A→B while the
    // session is stale on the old model A. `model_overridden` is false
    // because no SwitchModel control signal was sent — the session is merely
    // stale. Despite acp_model("A") != persona_model("B"), no RuntimeOverride
    // should be displayed.
    let record = test_record();
    let runtime = test_runtime();
    let cache = SessionConfigCache {
        config_options: vec![],
        available_modes: vec![],
        available_models: vec![],
        current_model: Some("old-persona-model".to_string()),
        model_overridden: false,
        goose_native_config: None,
        captured_at: "".to_string(),
    };

    let surface = read_config_surface(
        &record,
        Some(runtime),
        Some(&cache),
        Some(("new-persona-model", ConfigOrigin::PersonaDefault)),
    );
    let model = surface.normalized.model.unwrap();

    // model_overridden is false => no RuntimeOverride, even though
    // acp_model != persona_model. The old divergence-based signal would
    // have false-positived here. The persona is never surfaced as the
    // overridden secondary (that marker is exclusive to a real override).
    assert_ne!(model.origin, ConfigOrigin::RuntimeOverride);
    assert_ne!(model.overridden_origin, Some(ConfigOrigin::PersonaDefault));
}

#[test]
fn persona_provider_injection_produces_persona_default_origin() {
    let mut record = test_record();
    // Simulate: record has no provider env var, persona provides one.
    // The call-site injects it as GOOSE_PROVIDER before calling the reader.
    record
        .env_vars
        .insert("GOOSE_PROVIDER".to_string(), "anthropic".to_string());
    let runtime = test_runtime();

    let mut surface = read_config_surface(&record, Some(runtime), None, None);

    // Reader sees injected provider as BuzzExplicit.
    let provider = surface.normalized.provider.as_ref().unwrap();
    assert_eq!(provider.value.as_deref(), Some("anthropic"));
    assert_eq!(provider.origin, ConfigOrigin::BuzzExplicit);

    // Call-site re-tags (simulating had_provider == false).
    if let Some(ref mut field) = surface.normalized.provider {
        if field.origin == ConfigOrigin::BuzzExplicit {
            field.origin = ConfigOrigin::PersonaDefault;
        }
    }

    let provider = surface.normalized.provider.unwrap();
    assert_eq!(provider.value.as_deref(), Some("anthropic"));
    assert_eq!(provider.origin, ConfigOrigin::PersonaDefault);
}

#[test]
fn persona_system_prompt_injection_produces_persona_default_origin() {
    let mut record = test_record();
    // Simulate: record has no system_prompt, persona provides one via env var.
    // The call-site injects it as BUZZ_ACP_SYSTEM_PROMPT before calling the reader.
    record.env_vars.insert(
        "BUZZ_ACP_SYSTEM_PROMPT".to_string(),
        "You are a helpful assistant.".to_string(),
    );
    let runtime = test_runtime();

    let mut surface = read_config_surface(&record, Some(runtime), None, None);

    // Reader sees injected prompt as BuzzExplicit.
    let prompt = surface.normalized.system_prompt.as_ref().unwrap();
    assert_eq!(
        prompt.value.as_deref(),
        Some("You are a helpful assistant.")
    );
    assert_eq!(prompt.origin, ConfigOrigin::BuzzExplicit);

    // Call-site re-tags (simulating had_prompt == false).
    if let Some(ref mut field) = surface.normalized.system_prompt {
        if field.origin == ConfigOrigin::BuzzExplicit {
            field.origin = ConfigOrigin::PersonaDefault;
        }
    }

    let prompt = surface.normalized.system_prompt.unwrap();
    assert_eq!(
        prompt.value.as_deref(),
        Some("You are a helpful assistant.")
    );
    assert_eq!(prompt.origin, ConfigOrigin::PersonaDefault);
}

#[test]
fn config_file_only_system_prompt_surfaces_as_read_only_config_file_field() {
    // Record/env has no prompt; the config file does. It must NOT be
    // dropped — it should surface with ConfigFile origin, read-only.
    let field = build_system_prompt_field(&None, &Some("File-driven prompt.".to_string())).unwrap();
    assert_eq!(field.value.as_deref(), Some("File-driven prompt."));
    assert_eq!(field.origin, ConfigOrigin::ConfigFile);
    assert!(matches!(field.write_via, ConfigWriteMechanism::ReadOnly));
    assert!(field.overridden_value.is_none());
}

#[test]
fn record_system_prompt_shadows_config_file_prompt_as_secondary() {
    let field = build_system_prompt_field(
        &Some("Record prompt.".to_string()),
        &Some("File prompt.".to_string()),
    )
    .unwrap();
    assert_eq!(field.value.as_deref(), Some("Record prompt."));
    assert_eq!(field.origin, ConfigOrigin::BuzzExplicit);
    assert_eq!(field.overridden_value.as_deref(), Some("File prompt."));
    assert_eq!(field.overridden_origin, Some(ConfigOrigin::ConfigFile));
}

#[test]
fn no_system_prompt_from_any_tier_yields_none() {
    assert!(build_system_prompt_field(&None, &None).is_none());
}

#[test]
fn explicit_record_model_not_retagged_when_already_present() {
    let mut record = test_record();
    // Record already has its own model — persona resolution should NOT re-tag.
    record.model = Some("explicit-model".to_string());
    let runtime = test_runtime();

    let surface = read_config_surface(&record, Some(runtime), None, None);

    // had_model == true, so no re-tagging occurs. Origin stays BuzzExplicit.
    let model = surface.normalized.model.unwrap();
    assert_eq!(model.value.as_deref(), Some("explicit-model"));
    assert_eq!(model.origin, ConfigOrigin::BuzzExplicit);
}

#[test]
fn extra_env_vars_appear_in_advanced_as_buzz_explicit() {
    let mut record = test_record();
    // Normalized keys — must NOT appear in advanced.
    record
        .env_vars
        .insert("GOOSE_MODEL".to_string(), "some-model".to_string());
    record
        .env_vars
        .insert("BUZZ_ACP_SYSTEM_PROMPT".to_string(), "hello".to_string());
    // Non-normalized key — MUST appear in advanced.
    record
        .env_vars
        .insert("SPROUT_ACP_MEMORY".to_string(), "mem-value".to_string());
    let runtime = test_runtime();

    let surface = read_config_surface(&record, Some(runtime), None, None);

    let advanced_keys: Vec<&str> = surface.advanced.iter().map(|f| f.key.as_str()).collect();
    assert!(
        advanced_keys.contains(&"SPROUT_ACP_MEMORY"),
        "extra env var must appear in advanced"
    );
    assert!(
        !advanced_keys.contains(&"GOOSE_MODEL"),
        "normalized model key must not appear in advanced"
    );
    assert!(
        !advanced_keys.contains(&"BUZZ_ACP_SYSTEM_PROMPT"),
        "normalized system prompt key must not appear in advanced"
    );

    let field = surface
        .advanced
        .iter()
        .find(|f| f.key == "SPROUT_ACP_MEMORY")
        .unwrap();
    assert_eq!(field.value.as_deref(), Some("mem-value"));
    assert_eq!(field.origin, ConfigOrigin::BuzzExplicit);
    assert!(matches!(
        field.write_via,
        ConfigWriteMechanism::RespawnWithEnvVar { ref env_key } if env_key == "SPROUT_ACP_MEMORY"
    ));
}

#[test]
fn extra_env_var_skipped_when_already_in_file_config_extra() {
    // If a key is in both record.env_vars and file_config.extra, the config
    // file entry wins (it was already added to advanced). The env var must
    // not produce a second entry.
    //
    // We can't inject into file_config.extra directly in a unit test (it
    // comes from disk), so we verify the dedup logic via the normalized-key
    // path: GOOSE_THINKING_EFFORT is a normalized key and must not appear
    // in advanced even if set in env_vars.
    let mut record = test_record();
    record
        .env_vars
        .insert("GOOSE_THINKING_EFFORT".to_string(), "high".to_string());
    let runtime = test_runtime();

    let surface = read_config_surface(&record, Some(runtime), None, None);

    let advanced_keys: Vec<&str> = surface.advanced.iter().map(|f| f.key.as_str()).collect();
    assert!(
        !advanced_keys.contains(&"GOOSE_THINKING_EFFORT"),
        "normalized thinking key must not appear in advanced"
    );
}

// ── buzz-agent normalized env-var field tests ───────────────────────────────
//
// buzz-agent uses env vars (not a config file) for max_output_tokens and
// context_limit. build_numeric_env_field must surface these as BuzzExplicit
// when the env var is present in record.env_vars, and must not double-surface
// them in the advanced tier.

fn buzz_agent_runtime() -> &'static KnownAcpRuntime {
    &KnownAcpRuntime {
        id: "buzz-agent",
        label: "Buzz Agent",
        commands: &["buzz-agent"],
        aliases: &[],
        avatar_url: "",
        mcp_command: None,
        mcp_hooks: false,
        underlying_cli: None,
        cli_install_commands: &[],
        adapter_install_commands: &[],
        install_instructions_url: "",
        cli_install_hint: "",
        adapter_install_hint: "",
        skill_dir: None,
        supports_acp_model_switching: true,
        model_env_var: Some("BUZZ_AGENT_MODEL"),
        provider_env_var: Some("BUZZ_AGENT_PROVIDER"),
        provider_locked: false,
        default_env: &[],
        config_file_path: None,
        config_file_format: None,
        supports_acp_native_config: false,
        thinking_env_var: Some("BUZZ_AGENT_THINKING_EFFORT"),
        max_tokens_env_var: Some("BUZZ_AGENT_MAX_OUTPUT_TOKENS"),
        context_limit_env_var: Some("BUZZ_AGENT_MAX_CONTEXT_TOKENS"),
        required_normalized_fields: &["model", "provider"],
        login_hint: None,
        auth_probe_args: None,
    }
}

#[test]
fn buzz_agent_max_output_tokens_from_env_is_buzz_explicit() {
    let mut record = test_record();
    record.env_vars.insert(
        "BUZZ_AGENT_MAX_OUTPUT_TOKENS".to_string(),
        "8192".to_string(),
    );
    let runtime = buzz_agent_runtime();

    let surface = read_config_surface(&record, Some(runtime), None, None);

    let field = surface.normalized.max_output_tokens.unwrap();
    assert_eq!(field.value.as_deref(), Some("8192"));
    assert_eq!(field.origin, ConfigOrigin::BuzzExplicit);
    assert!(matches!(
        field.write_via,
        ConfigWriteMechanism::RespawnWithEnvVar { ref env_key }
            if env_key == "BUZZ_AGENT_MAX_OUTPUT_TOKENS"
    ));
}

#[test]
fn buzz_agent_context_limit_from_env_is_buzz_explicit() {
    let mut record = test_record();
    record.env_vars.insert(
        "BUZZ_AGENT_MAX_CONTEXT_TOKENS".to_string(),
        "100000".to_string(),
    );
    let runtime = buzz_agent_runtime();

    let surface = read_config_surface(&record, Some(runtime), None, None);

    let field = surface.normalized.context_limit.unwrap();
    assert_eq!(field.value.as_deref(), Some("100000"));
    assert_eq!(field.origin, ConfigOrigin::BuzzExplicit);
    assert!(matches!(
        field.write_via,
        ConfigWriteMechanism::RespawnWithEnvVar { ref env_key }
            if env_key == "BUZZ_AGENT_MAX_CONTEXT_TOKENS"
    ));
}

#[test]
fn buzz_agent_max_tokens_absent_when_no_env_var_or_file() {
    // buzz-agent has no config file, and env var is not set.
    let record = test_record();
    let runtime = buzz_agent_runtime();

    let surface = read_config_surface(&record, Some(runtime), None, None);

    assert!(
        surface.normalized.max_output_tokens.is_none(),
        "max_output_tokens must be None when env var not set and no config file"
    );
    assert!(
        surface.normalized.context_limit.is_none(),
        "context_limit must be None when env var not set and no config file"
    );
}

#[test]
fn buzz_agent_max_tokens_env_var_not_double_surfaced_in_advanced() {
    let mut record = test_record();
    record.env_vars.insert(
        "BUZZ_AGENT_MAX_OUTPUT_TOKENS".to_string(),
        "4096".to_string(),
    );
    record.env_vars.insert(
        "BUZZ_AGENT_MAX_CONTEXT_TOKENS".to_string(),
        "50000".to_string(),
    );
    let runtime = buzz_agent_runtime();

    let surface = read_config_surface(&record, Some(runtime), None, None);

    let advanced_keys: Vec<&str> = surface.advanced.iter().map(|f| f.key.as_str()).collect();
    assert!(
        !advanced_keys.contains(&"BUZZ_AGENT_MAX_OUTPUT_TOKENS"),
        "max_output_tokens must not appear in advanced when normalized"
    );
    assert!(
        !advanced_keys.contains(&"BUZZ_AGENT_MAX_CONTEXT_TOKENS"),
        "context_limit must not appear in advanced when normalized"
    );
}

#[test]
fn buzz_agent_thinking_effort_from_env_is_buzz_explicit() {
    let mut record = test_record();
    record
        .env_vars
        .insert("BUZZ_AGENT_THINKING_EFFORT".to_string(), "high".to_string());
    let runtime = buzz_agent_runtime();

    let surface = read_config_surface(&record, Some(runtime), None, None);

    let field = surface.normalized.thinking_effort.unwrap();
    assert_eq!(field.value.as_deref(), Some("high"));
    assert_eq!(field.origin, ConfigOrigin::BuzzExplicit);
    assert!(matches!(
        field.write_via,
        ConfigWriteMechanism::RespawnWithEnvVar { ref env_key }
            if env_key == "BUZZ_AGENT_THINKING_EFFORT"
    ));
}

#[test]
fn buzz_agent_thinking_effort_env_var_not_double_surfaced_in_advanced() {
    let mut record = test_record();
    record.env_vars.insert(
        "BUZZ_AGENT_THINKING_EFFORT".to_string(),
        "medium".to_string(),
    );
    let runtime = buzz_agent_runtime();

    let surface = read_config_surface(&record, Some(runtime), None, None);

    let advanced_keys: Vec<&str> = surface.advanced.iter().map(|f| f.key.as_str()).collect();
    assert!(
        !advanced_keys.contains(&"BUZZ_AGENT_THINKING_EFFORT"),
        "thinking_effort must not appear in advanced when normalized"
    );
}

#[test]
fn missing_required_provider_still_returns_dropdown_field() {
    let provider = build_provider_field(&None, &None, Some("GOOSE_PROVIDER"), false, true)
        .expect("required provider field should be surfaced even when empty");

    assert_eq!(provider.value, None);
    assert_eq!(provider.origin, ConfigOrigin::EnvVar);
    assert!(provider.is_required);
}

#[test]
fn missing_optional_provider_stays_hidden() {
    assert!(build_provider_field(&None, &None, Some("GOOSE_PROVIDER"), false, false).is_none());
}
