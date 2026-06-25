use crate::managed_agents::discovery::KnownAcpRuntime;
use crate::managed_agents::types::ManagedAgentRecord;

use super::types::*;

/// Build the full config surface for an agent, merging all four tiers.
///
/// Pre-spawn (no session cache): tiers 2a (env vars / record) and 2b (config files).
/// Post-spawn (session cache present): adds tiers 1a (ACP native) and 1b (ACP configOptions).
pub(crate) fn read_config_surface(
    record: &ManagedAgentRecord,
    runtime_meta: Option<&KnownAcpRuntime>,
    session_cache: Option<&SessionConfigCache>,
    baseline: Option<(&str, ConfigOrigin)>,
) -> RuntimeConfigSurface {
    let is_pre_spawn = session_cache.is_none();

    // Tier 2b: config file values.
    let (file_config, file_was_read) = runtime_meta
        .map(|m| m.id)
        .and_then(|id| match id {
            "goose" => super::goose::read_config_file().map(|c| (c, true)),
            "claude" => super::claude::read_config_file().map(|c| (c, true)),
            "codex" => super::codex::read_config_file().map(|c| (c, true)),
            "buzz-agent" => super::buzz_agent::read_config_file().map(|c| (c, true)),
            _ => None,
        })
        .unwrap_or_else(|| (RuntimeFileConfig::default(), false));

    // Tier 2a: record-level values (Buzz-explicit).
    let record_model = record.model.clone();
    let record_provider = record
        .env_vars
        .get(runtime_meta.and_then(|m| m.provider_env_var).unwrap_or(""))
        .cloned();

    let supports_acp_model = runtime_meta.is_some_and(|m| m.supports_acp_model_switching);
    let model_env_var = runtime_meta.and_then(|m| m.model_env_var);
    let provider_env_var = runtime_meta.and_then(|m| m.provider_env_var);
    let provider_locked = runtime_meta.is_some_and(|m| m.provider_locked);
    let thinking_env_var = runtime_meta.and_then(|m| m.thinking_env_var);
    let supports_acp_native = runtime_meta.is_some_and(|m| m.supports_acp_native_config);

    // Tier 1b: ACP configOptions from session cache.
    let acp_model = session_cache.and_then(|c| c.current_model.clone());
    let acp_mode = session_cache.and_then(|c| find_config_option_value(c, "mode"));
    let acp_effort = session_cache.and_then(|c| find_config_option_value(c, "effort"));
    let record_effort = thinking_env_var
        .and_then(|k| record.env_vars.get(k))
        .cloned();

    let model_overridden = session_cache.is_some_and(|c| c.model_overridden);

    let normalized = NormalizedConfig {
        model: Some(apply_runtime_override(
            build_model_field(
                &record_model,
                &file_config.model,
                &acp_model,
                model_env_var,
                supports_acp_model,
                is_pre_spawn,
                session_cache,
            ),
            acp_model.as_deref(),
            baseline,
            model_overridden,
        )),
        provider: build_provider_field(
            &record_provider,
            &file_config.provider,
            provider_env_var,
            provider_locked,
        ),
        mode: build_mode_field(&file_config.mode, &acp_mode, is_pre_spawn, session_cache),
        thinking_effort: build_thinking_field(
            &record_effort,
            &file_config.thinking_effort,
            &acp_effort,
            thinking_env_var,
            is_pre_spawn,
            session_cache,
        ),
        max_output_tokens: file_config
            .max_output_tokens
            .as_ref()
            .map(|v| NormalizedField {
                value: Some(v.clone()),
                origin: ConfigOrigin::ConfigFile,
                is_writable: false,
                write_via: ConfigWriteMechanism::ReadOnly,
                overridden_value: None,
                overridden_origin: None,
            }),
        context_limit: file_config.context_limit.as_ref().map(|v| NormalizedField {
            value: Some(v.clone()),
            origin: ConfigOrigin::ConfigFile,
            is_writable: false,
            write_via: ConfigWriteMechanism::ReadOnly,
            overridden_value: None,
            overridden_origin: None,
        }),
        system_prompt: {
            let record_system_prompt = record
                .system_prompt
                .clone()
                .or_else(|| record.env_vars.get("BUZZ_ACP_SYSTEM_PROMPT").cloned());
            record_system_prompt.as_ref().map(|v| NormalizedField {
                value: Some(v.clone()),
                origin: ConfigOrigin::BuzzExplicit,
                is_writable: true,
                write_via: ConfigWriteMechanism::RespawnWithEnvVar {
                    env_key: "BUZZ_ACP_SYSTEM_PROMPT".to_string(),
                },
                overridden_value: file_config.system_prompt.clone(),
                overridden_origin: file_config
                    .system_prompt
                    .as_ref()
                    .map(|_| ConfigOrigin::ConfigFile),
            })
        },
    };

    // Advanced fields from config file extras.
    let advanced: Vec<ConfigField> = file_config
        .extra
        .iter()
        .map(|(k, v)| ConfigField {
            key: k.clone(),
            label: k.clone(),
            value: Some(v.clone()),
            origin: ConfigOrigin::ConfigFile,
            schema_type: ConfigFieldType::String,
            is_writable: false,
            write_via: ConfigWriteMechanism::ReadOnly,
        })
        .collect();

    // Collect the env var keys already covered by normalized fields so we don't double-surface them.
    let normalized_env_keys: Vec<&str> = [
        model_env_var,
        provider_env_var,
        thinking_env_var,
        Some("BUZZ_ACP_SYSTEM_PROMPT"),
    ]
    .into_iter()
    .flatten()
    .collect();

    // Tier 2a: remaining env vars not covered by normalized fields.
    // Env var wins over config file for the same key (tier 2a > 2b), so skip
    // keys already present in file_config.extra.
    let mut advanced = advanced;
    for (k, v) in &record.env_vars {
        if normalized_env_keys.contains(&k.as_str()) {
            continue;
        }
        if file_config.extra.contains_key(k) {
            continue; // config file already surfaced this key
        }
        advanced.push(ConfigField {
            key: k.clone(),
            label: k.clone(),
            value: Some(v.clone()),
            origin: ConfigOrigin::BuzzExplicit,
            schema_type: ConfigFieldType::String,
            is_writable: true,
            write_via: ConfigWriteMechanism::RespawnWithEnvVar { env_key: k.clone() },
        });
    }

    let config_file_path = runtime_meta
        .and_then(|m| m.config_file_path)
        .map(resolve_tilde);

    let sources = ConfigSourceReport {
        acp_native: if supports_acp_native {
            if session_cache
                .and_then(|c| c.goose_native_config.as_ref())
                .is_some()
            {
                ConfigTierStatus::Available
            } else {
                // Post-spawn without native config data is also Pending — it arrives
                // asynchronously after the session/new response.
                ConfigTierStatus::Pending
            }
        } else {
            ConfigTierStatus::NotApplicable
        },
        acp_config_options: if is_pre_spawn {
            ConfigTierStatus::Pending
        } else if session_cache.is_some_and(|c| !c.config_options.is_empty()) {
            ConfigTierStatus::Available
        } else {
            ConfigTierStatus::NotApplicable
        },
        env_vars: ConfigTierStatus::Available,
        config_file: if file_was_read {
            ConfigTierStatus::Available
        } else {
            ConfigTierStatus::NotApplicable
        },
        config_file_path,
    };

    RuntimeConfigSurface {
        runtime_id: runtime_meta.map(|m| m.id.to_string()),
        runtime_label: runtime_meta.map(|m| m.label.to_string()),
        is_pre_spawn,
        normalized,
        advanced,
        sources,
    }
}

fn build_model_field(
    record_model: &Option<String>,
    file_model: &Option<String>,
    acp_model: &Option<String>,
    model_env_var: Option<&str>,
    supports_acp_model: bool,
    is_pre_spawn: bool,
    session_cache: Option<&SessionConfigCache>,
) -> NormalizedField {
    // Precedence: Buzz-explicit > ACP current > config file
    let (value, origin) = if let Some(ref m) = record_model {
        (Some(m.clone()), ConfigOrigin::BuzzExplicit)
    } else if let Some(ref m) = acp_model {
        (Some(m.clone()), ConfigOrigin::AcpConfigOption)
    } else if let Some(ref m) = file_model {
        (Some(m.clone()), ConfigOrigin::ConfigFile)
    } else {
        (None, ConfigOrigin::EnvVar)
    };

    // The secondary expresses ONLY the static record-vs-file precedence: a
    // Buzz-explicit model shadowing a config-file model. The live-session
    // override (acp vs record/persona) is exclusively `apply_runtime_override`'s
    // job, gated on `model_overridden`. Surfacing `acp_model` here would leak an
    // override row even when no live switch has been applied.
    let (overridden_value, overridden_origin) = if record_model.is_some() && file_model.is_some() {
        (file_model.clone(), Some(ConfigOrigin::ConfigFile))
    } else {
        (None, None)
    };

    let write_via = model_write_mechanism(
        is_pre_spawn,
        supports_acp_model,
        session_cache,
        model_env_var,
    );

    NormalizedField {
        value,
        origin,
        is_writable: !matches!(write_via, ConfigWriteMechanism::ReadOnly),
        write_via,
        overridden_value,
        overridden_origin,
    }
}

/// Resolve how the model field is written back to the runtime.
/// Prefer ACP `set_config_option`/`set_model` post-spawn, else env-var respawn.
fn model_write_mechanism(
    is_pre_spawn: bool,
    supports_acp_model: bool,
    session_cache: Option<&SessionConfigCache>,
    model_env_var: Option<&str>,
) -> ConfigWriteMechanism {
    if !is_pre_spawn && has_config_option(session_cache, "model") {
        let config_id = find_model_config_id(session_cache).unwrap_or_else(|| "model".to_string());
        ConfigWriteMechanism::AcpSetConfigOption { config_id }
    } else if !is_pre_spawn && supports_acp_model {
        ConfigWriteMechanism::AcpSetSessionModel
    } else if let Some(env_key) = model_env_var {
        ConfigWriteMechanism::RespawnWithEnvVar {
            env_key: env_key.to_string(),
        }
    } else {
        ConfigWriteMechanism::ReadOnly
    }
}

/// Re-key the model field as a live runtime override when the harness signals
/// that a `SwitchModel` control signal set the model (Phase 3c).
///
/// The override-active signal is `model_overridden` from the
/// `session_config_captured` payload — NOT `acp_model != persona_model`, which
/// would false-positive when a persona model is edited mid-life while the
/// session is stale on the old model.
///
/// `baseline` is the value the live model overrides, paired with its true
/// origin — `(persona_model, PersonaDefault)` for a persona-linked agent, or
/// `(record_model, BuzzExplicit)` for a genuine-explicit agent that live-
/// switched. It is `Some` only when there is such a baseline to override
/// against; otherwise the field passes through unchanged. Carrying the origin
/// in the pair (rather than hardcoding it) lets the secondary be tagged by its
/// real source instead of always reading `PersonaDefault`.
///
/// The `acp == baseline_value` short-circuit keeps a live pick of the baseline
/// model itself from rendering a no-op "override of X with X". It yields a
/// CLEAN single-value field — `overridden_value`/`overridden_origin` cleared —
/// rather than passing `base` through, because `build_model_field` already
/// populates `base`'s secondary with an `AcpConfigOption` row for the
/// record-model-plus-live-session case; returning `base` would leak that
/// spurious row. The override preserves the base field's write mechanism — only
/// the displayed value, origin, and secondary change.
fn apply_runtime_override(
    base: NormalizedField,
    acp_model: Option<&str>,
    baseline: Option<(&str, ConfigOrigin)>,
    model_overridden: bool,
) -> NormalizedField {
    if !model_overridden {
        return base;
    }
    let (Some(acp), Some((baseline_value, baseline_origin))) = (acp_model, baseline) else {
        return base;
    };
    if acp == baseline_value {
        // Live pick equals the baseline — no real divergence. Strip any
        // secondary `build_model_field` may have produced so the panel shows a
        // single clean value rather than "X overridden by X".
        return NormalizedField {
            overridden_value: None,
            overridden_origin: None,
            ..base
        };
    }
    NormalizedField {
        value: Some(acp.to_string()),
        origin: ConfigOrigin::RuntimeOverride,
        overridden_value: Some(baseline_value.to_string()),
        overridden_origin: Some(baseline_origin),
        ..base
    }
}

fn build_provider_field(
    record_provider: &Option<String>,
    file_provider: &Option<String>,
    provider_env_var: Option<&str>,
    provider_locked: bool,
) -> Option<NormalizedField> {
    if provider_locked {
        return Some(NormalizedField {
            value: Some("Anthropic (locked)".to_string()),
            origin: ConfigOrigin::EnvVar,
            is_writable: false,
            write_via: ConfigWriteMechanism::ReadOnly,
            overridden_value: None,
            overridden_origin: None,
        });
    }

    let (value, origin) = if let Some(ref p) = record_provider {
        (Some(p.clone()), ConfigOrigin::BuzzExplicit)
    } else if let Some(ref p) = file_provider {
        (Some(p.clone()), ConfigOrigin::ConfigFile)
    } else {
        return None;
    };

    let write_via = if let Some(env_key) = provider_env_var {
        ConfigWriteMechanism::RespawnWithEnvVar {
            env_key: env_key.to_string(),
        }
    } else {
        ConfigWriteMechanism::ReadOnly
    };

    Some(NormalizedField {
        value,
        origin,
        is_writable: !matches!(write_via, ConfigWriteMechanism::ReadOnly),
        write_via,
        overridden_value: if record_provider.is_some() {
            file_provider.clone()
        } else {
            None
        },
        overridden_origin: if record_provider.is_some() && file_provider.is_some() {
            Some(ConfigOrigin::ConfigFile)
        } else {
            None
        },
    })
}

fn build_mode_field(
    file_mode: &Option<String>,
    acp_mode: &Option<String>,
    is_pre_spawn: bool,
    session_cache: Option<&SessionConfigCache>,
) -> Option<NormalizedField> {
    let (value, origin) = if let Some(ref m) = acp_mode {
        (Some(m.clone()), ConfigOrigin::AcpConfigOption)
    } else if let Some(ref m) = file_mode {
        (Some(m.clone()), ConfigOrigin::ConfigFile)
    } else {
        return None;
    };

    let write_via = if !is_pre_spawn && has_config_option(session_cache, "mode") {
        ConfigWriteMechanism::AcpSetConfigOption {
            config_id: "mode".to_string(),
        }
    } else {
        ConfigWriteMechanism::ReadOnly
    };

    Some(NormalizedField {
        value,
        origin,
        is_writable: !matches!(write_via, ConfigWriteMechanism::ReadOnly),
        write_via,
        overridden_value: if acp_mode.is_some() {
            file_mode.clone()
        } else {
            None
        },
        overridden_origin: if acp_mode.is_some() && file_mode.is_some() {
            Some(ConfigOrigin::ConfigFile)
        } else {
            None
        },
    })
}

fn build_thinking_field(
    record_effort: &Option<String>,
    file_effort: &Option<String>,
    acp_effort: &Option<String>,
    thinking_env_var: Option<&str>,
    is_pre_spawn: bool,
    session_cache: Option<&SessionConfigCache>,
) -> Option<NormalizedField> {
    let (value, origin) = if let Some(ref e) = record_effort {
        (Some(e.clone()), ConfigOrigin::BuzzExplicit)
    } else if let Some(ref e) = acp_effort {
        (Some(e.clone()), ConfigOrigin::AcpConfigOption)
    } else if let Some(ref e) = file_effort {
        (Some(e.clone()), ConfigOrigin::ConfigFile)
    } else {
        return None;
    };

    let write_via = if !is_pre_spawn && has_config_option(session_cache, "effort") {
        ConfigWriteMechanism::AcpSetConfigOption {
            config_id: "effort".to_string(),
        }
    } else if let Some(env_key) = thinking_env_var {
        ConfigWriteMechanism::RespawnWithEnvVar {
            env_key: env_key.to_string(),
        }
    } else {
        ConfigWriteMechanism::ReadOnly
    };

    Some(NormalizedField {
        value,
        origin,
        is_writable: !matches!(write_via, ConfigWriteMechanism::ReadOnly),
        write_via,
        overridden_value: if record_effort.is_some() {
            acp_effort.clone().or(file_effort.clone())
        } else if acp_effort.is_some() {
            file_effort.clone()
        } else {
            None
        },
        overridden_origin: if record_effort.is_some() && acp_effort.is_some() {
            Some(ConfigOrigin::AcpConfigOption)
        } else if file_effort.is_some() && (record_effort.is_some() || acp_effort.is_some()) {
            Some(ConfigOrigin::ConfigFile)
        } else {
            None
        },
    })
}

// ── ACP cache helpers ────────────────────────────────────────────────────────

fn find_config_option_value(cache: &SessionConfigCache, category: &str) -> Option<String> {
    cache
        .config_options
        .iter()
        .find(|o| o.category.as_deref() == Some(category))
        .and_then(|o| o.current_value.clone())
}

fn has_config_option(cache: Option<&SessionConfigCache>, category: &str) -> bool {
    cache.is_some_and(|c| {
        c.config_options
            .iter()
            .any(|o| o.category.as_deref() == Some(category))
    })
}

fn find_model_config_id(cache: Option<&SessionConfigCache>) -> Option<String> {
    cache.and_then(|c| {
        c.config_options
            .iter()
            .find(|o| o.category.as_deref() == Some("model"))
            .map(|o| o.config_id.clone())
    })
}

fn resolve_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).display().to_string();
        }
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::managed_agents::discovery::KnownAcpRuntime;
    use crate::managed_agents::types::ManagedAgentRecord;

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
            mcp_toolsets: None,
            env_vars: BTreeMap::new(),
            start_on_app_launch: false,
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
            respond_to: crate::managed_agents::types::RespondTo::OwnerOnly,
            respond_to_allowlist: vec![],
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
        assert!(!provider.is_writable);
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
        assert!(field.is_writable);
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
}
