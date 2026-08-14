use crate::managed_agents::discovery::KnownAcpRuntime;
use crate::managed_agents::types::ManagedAgentRecord;

use super::types::*;

/// Build the full config surface for an agent, merging all tiers.
///
/// Inherited values flow through `tiers` — a sanitized snapshot of the
/// persona and global tiers assembled at the command boundary. Each field
/// builder constructs its own candidate list and resolves via
/// `resolve_with_override`.
pub(crate) fn read_config_surface(
    record: &ManagedAgentRecord,
    runtime_meta: Option<&KnownAcpRuntime>,
    session_cache: Option<&SessionConfigCache>,
    tiers: &InheritedConfigTiers,
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

    // Runtime-specific env var keys.
    let supports_acp_model = runtime_meta.is_some_and(|m| m.supports_acp_model_switching);
    let model_env_var = runtime_meta.and_then(|m| m.model_env_var);
    let provider_env_var = runtime_meta.and_then(|m| m.provider_env_var);
    let provider_locked = runtime_meta.is_some_and(|m| m.provider_locked);
    let thinking_env_var = runtime_meta.and_then(|m| m.thinking_env_var);
    let supports_acp_native = runtime_meta.is_some_and(|m| m.supports_acp_native_config);
    let required_fields: &[&str] = runtime_meta
        .map(|m| m.required_normalized_fields)
        .unwrap_or(&[]);
    let max_tokens_env_var = runtime_meta.and_then(|m| m.max_tokens_env_var);
    let context_limit_env_var = runtime_meta.and_then(|m| m.context_limit_env_var);

    // Tier 1b: ACP configOptions from session cache.
    let acp_model = session_cache.and_then(|c| {
        c.current_model
            .clone()
            .or_else(|| find_config_option_value(c, "model"))
    });
    let acp_mode = session_cache.and_then(|c| find_config_option_value(c, "mode"));
    let acp_effort = session_cache.and_then(|c| find_config_option_value(c, "effort"));

    let model_overridden = session_cache.is_some_and(|c| c.model_overridden);

    let normalized = NormalizedConfig {
        model: Some(build_model_field(
            record,
            &file_config.model,
            &acp_model,
            model_env_var,
            supports_acp_model,
            is_pre_spawn,
            session_cache,
            required_fields.contains(&"model"),
            model_overridden,
            tiers,
        )),
        provider: build_provider_field(
            record,
            &file_config.provider,
            provider_env_var,
            provider_locked,
            required_fields.contains(&"provider"),
            tiers,
        ),
        mode: build_mode_field(&file_config.mode, &acp_mode, is_pre_spawn, session_cache),
        thinking_effort: build_thinking_field(
            record,
            &file_config.thinking_effort,
            &acp_effort,
            thinking_env_var,
            is_pre_spawn,
            session_cache,
            tiers,
        ),
        max_output_tokens: build_numeric_env_field(
            max_tokens_env_var,
            record,
            &file_config.max_output_tokens,
            tiers,
        ),
        context_limit: build_numeric_env_field(
            context_limit_env_var,
            record,
            &file_config.context_limit,
            tiers,
        ),
        system_prompt: build_system_prompt_field(record, &file_config.system_prompt, tiers),
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
            write_via: ConfigWriteMechanism::ReadOnly,
        })
        .collect();

    // Collect the env var keys already covered by normalized fields.
    let normalized_env_keys: Vec<&str> = [
        model_env_var,
        provider_env_var,
        thinking_env_var,
        max_tokens_env_var,
        context_limit_env_var,
        Some("BUZZ_ACP_SYSTEM_PROMPT"),
    ]
    .into_iter()
    .flatten()
    .collect();

    // Tier 2a: remaining env vars not covered by normalized fields.
    let mut advanced = advanced;
    for (k, v) in &record.env_vars {
        if normalized_env_keys.contains(&k.as_str()) {
            continue;
        }
        if file_config.extra.contains_key(k) {
            continue;
        }
        advanced.push(ConfigField {
            key: k.clone(),
            label: k.clone(),
            value: Some(v.clone()),
            origin: ConfigOrigin::BuzzExplicit,
            schema_type: ConfigFieldType::String,
            write_via: ConfigWriteMechanism::RespawnWithEnvVar { env_key: k.clone() },
        });
    }

    let config_file_path = runtime_meta
        .and_then(|m| m.config_file_path)
        .map(resolve_tilde);
    let mcp_config_file_path = runtime_meta.and_then(mcp_config_file_path_for_runtime);
    let extensions = file_config.extensions.clone();

    let sources = ConfigSourceReport {
        acp_native: if supports_acp_native {
            if session_cache
                .and_then(|c| c.goose_native_config.as_ref())
                .is_some()
            {
                ConfigTierStatus::Available
            } else {
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
        mcp_config_file_path,
    };

    RuntimeConfigSurface {
        runtime_id: runtime_meta.map(|m| m.id.to_string()),
        runtime_label: runtime_meta.map(|m| m.label.to_string()),
        is_pre_spawn,
        normalized,
        advanced,
        extensions,
        sources,
    }
}

fn mcp_config_file_path_for_runtime(runtime: &KnownAcpRuntime) -> Option<String> {
    match runtime.id {
        "goose" => {
            super::goose::goose_config_path().map(|path| path.to_string_lossy().into_owned())
        }
        "claude" => Some(resolve_tilde("~/.claude.json")),
        "codex" => {
            super::codex::codex_config_path().map(|path| path.to_string_lossy().into_owned())
        }
        _ => None,
    }
}

/// Extract an env-backed candidate value for `env_key` from each tier in
/// spawn precedence: record env > persona env > global env > definition env.
/// Returns `[record, persona, global, definition]` — `None` when key is absent.
fn env_candidates<'a>(
    env_key: &str,
    record_env: &'a std::collections::BTreeMap<String, String>,
    persona_env: &'a std::collections::BTreeMap<String, String>,
    global_env: &'a std::collections::BTreeMap<String, String>,
    definition_env: &'a std::collections::BTreeMap<String, String>,
) -> [Option<&'a str>; 4] {
    [
        record_env.get(env_key).map(String::as_str),
        persona_env.get(env_key).map(String::as_str),
        global_env.get(env_key).map(String::as_str),
        definition_env.get(env_key).map(String::as_str),
    ]
}

#[allow(clippy::too_many_arguments)]
fn build_model_field(
    record: &ManagedAgentRecord,
    file_model: &Option<String>,
    acp_model: &Option<String>,
    model_env_var: Option<&str>,
    supports_acp_model: bool,
    is_pre_spawn: bool,
    session_cache: Option<&SessionConfigCache>,
    is_required: bool,
    model_overridden: bool,
    tiers: &InheritedConfigTiers,
) -> NormalizedField {
    let [rec_env, pers_env, glob_env, def_env] = model_env_var
        .map(|k| {
            env_candidates(
                k,
                &record.env_vars,
                &tiers.persona_env,
                &tiers.global_env,
                &tiers.definition_env,
            )
        })
        .unwrap_or([None, None, None, None]);

    // Structured record model (definition-less only; linked cleared upstream).
    let struct_record = record.model.as_deref();
    let struct_persona = tiers.persona_model.as_deref();
    let struct_global = tiers.global_model.as_deref();

    // Configured candidates in spawn order: record env > persona env > global env >
    // definition env > struct record > struct persona > struct global > file.
    // The file entry is always last; everything before it is a "configured" candidate
    // that gates whether ACP participates as a fallback (see any_configured below).
    let configured: &[(Option<&str>, ConfigOrigin)] = &[
        (rec_env, ConfigOrigin::BuzzExplicit),
        (pers_env, ConfigOrigin::PersonaDefault),
        (glob_env, ConfigOrigin::GlobalDefault),
        (def_env, ConfigOrigin::HarnessDefault),
        (struct_record, ConfigOrigin::BuzzExplicit),
        (struct_persona, ConfigOrigin::PersonaDefault),
        (struct_global, ConfigOrigin::GlobalDefault),
        (file_model.as_deref(), ConfigOrigin::ConfigFile),
    ];
    // "Configured" = any non-file candidate. The file entry is always last, so
    // slicing to len()-1 is equivalent to the old magic `[..6]` and stays correct
    // if the array ever grows again.
    let any_configured = configured[..configured.len() - 1]
        .iter()
        .any(|(v, _)| v.is_some());

    // When model_overridden is true and ACP is present, ACP is the live winner.
    // The top configured candidate becomes the secondary (the overridden baseline).
    // Equal-value case: ACP == baseline → fall through to normal resolution so
    // the field carries the correct baseline origin rather than RuntimeOverride.
    if model_overridden {
        if let Some(acp) = acp_model.as_deref() {
            let baseline = configured.iter().find(|(v, _)| v.is_some());
            match baseline {
                Some((Some(baseline_value), _)) if acp == *baseline_value => {
                    // Equal-value switch: no real divergence.
                    // Fall through to the normal resolve path below — it will
                    // return the same value with its true baseline origin, with
                    // no secondary row.
                }
                Some((Some(baseline_value), baseline_origin)) => {
                    return NormalizedField {
                        value: Some(acp.to_string()),
                        origin: ConfigOrigin::RuntimeOverride,
                        write_via: model_write_mechanism(
                            is_pre_spawn,
                            supports_acp_model,
                            session_cache,
                            model_env_var,
                        ),
                        overridden_value: Some(baseline_value.to_string()),
                        overridden_origin: Some(baseline_origin.clone()),
                        is_required,
                    };
                }
                _ => {
                    // No configured baseline — ACP is the only source.
                    return NormalizedField {
                        value: Some(acp.to_string()),
                        origin: ConfigOrigin::RuntimeOverride,
                        write_via: model_write_mechanism(
                            is_pre_spawn,
                            supports_acp_model,
                            session_cache,
                            model_env_var,
                        ),
                        overridden_value: None,
                        overridden_origin: None,
                        is_required,
                    };
                }
            }
        }
    }

    let (value, origin, overridden_value, overridden_origin) = if !any_configured {
        // No configured candidate: ACP participates as AcpConfigOption fallback.
        let full: &[(Option<&str>, ConfigOrigin)] = &[
            (acp_model.as_deref(), ConfigOrigin::AcpConfigOption),
            (file_model.as_deref(), ConfigOrigin::ConfigFile),
        ];
        resolve_with_override(full).unwrap_or((None, ConfigOrigin::EnvVar, None, None))
    } else {
        // ACP excluded: a configured value is pending and wins over live ACP.
        match resolve_with_override(configured) {
            Some(r) => r,
            None => (None, ConfigOrigin::EnvVar, None, None),
        }
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
        write_via,
        overridden_value,
        overridden_origin,
        is_required,
    }
}

/// Resolve how the model field is written back to the runtime.
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

fn build_provider_field(
    record: &ManagedAgentRecord,
    file_provider: &Option<String>,
    provider_env_var: Option<&str>,
    provider_locked: bool,
    is_required: bool,
    tiers: &InheritedConfigTiers,
) -> Option<NormalizedField> {
    if provider_locked {
        return Some(NormalizedField {
            value: Some("Anthropic (locked)".to_string()),
            origin: ConfigOrigin::HarnessConstraint,
            write_via: ConfigWriteMechanism::ReadOnly,
            overridden_value: None,
            overridden_origin: None,
            is_required: false,
        });
    }

    let [rec_env, pers_env, glob_env, def_env] = provider_env_var
        .map(|k| {
            env_candidates(
                k,
                &record.env_vars,
                &tiers.persona_env,
                &tiers.global_env,
                &tiers.definition_env,
            )
        })
        .unwrap_or([None, None, None, None]);

    let struct_record = record.provider.as_deref();

    let tiers_list: &[(Option<&str>, ConfigOrigin)] = &[
        (rec_env, ConfigOrigin::BuzzExplicit),
        (pers_env, ConfigOrigin::PersonaDefault),
        (glob_env, ConfigOrigin::GlobalDefault),
        (def_env, ConfigOrigin::HarnessDefault),
        (struct_record, ConfigOrigin::BuzzExplicit),
        (
            tiers.persona_provider.as_deref(),
            ConfigOrigin::PersonaDefault,
        ),
        (
            tiers.global_provider.as_deref(),
            ConfigOrigin::GlobalDefault,
        ),
        (file_provider.as_deref(), ConfigOrigin::ConfigFile),
    ];

    let (value, origin, overridden_value, overridden_origin) =
        match resolve_with_override(tiers_list) {
            Some(resolved) => resolved,
            None if is_required => (None, ConfigOrigin::EnvVar, None, None),
            None => return None,
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
        write_via,
        overridden_value,
        overridden_origin,
        is_required,
    })
}

fn build_mode_field(
    file_mode: &Option<String>,
    acp_mode: &Option<String>,
    is_pre_spawn: bool,
    session_cache: Option<&SessionConfigCache>,
) -> Option<NormalizedField> {
    let tiers: &[(Option<&str>, ConfigOrigin)] = &[
        (acp_mode.as_deref(), ConfigOrigin::AcpConfigOption),
        (file_mode.as_deref(), ConfigOrigin::ConfigFile),
    ];
    let (value, origin, overridden_value, overridden_origin) = resolve_with_override(tiers)?;

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
        write_via,
        overridden_value,
        overridden_origin,
        is_required: false,
    })
}

#[allow(clippy::too_many_arguments)]
fn build_thinking_field(
    record: &ManagedAgentRecord,
    file_effort: &Option<String>,
    acp_effort: &Option<String>,
    thinking_env_var: Option<&str>,
    is_pre_spawn: bool,
    session_cache: Option<&SessionConfigCache>,
    tiers: &InheritedConfigTiers,
) -> Option<NormalizedField> {
    // Tier ordering: record env > ACP > persona env > global env > definition env > config file.
    let [rec_env, pers_env, glob_env, def_env] = thinking_env_var
        .map(|k| {
            env_candidates(
                k,
                &record.env_vars,
                &tiers.persona_env,
                &tiers.global_env,
                &tiers.definition_env,
            )
        })
        .unwrap_or([None, None, None, None]);

    let tiers_list: &[(Option<&str>, ConfigOrigin)] = &[
        (rec_env, ConfigOrigin::BuzzExplicit),
        (acp_effort.as_deref(), ConfigOrigin::AcpConfigOption),
        (pers_env, ConfigOrigin::PersonaDefault),
        (glob_env, ConfigOrigin::GlobalDefault),
        (def_env, ConfigOrigin::HarnessDefault),
        (file_effort.as_deref(), ConfigOrigin::ConfigFile),
    ];
    let (value, origin, overridden_value, overridden_origin) = resolve_with_override(tiers_list)?;

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
        write_via,
        overridden_value,
        overridden_origin,
        is_required: false,
    })
}

/// Numeric fields (max_output_tokens, context_limit).
/// Tier ordering: record env > persona env > global env > config file.
fn build_numeric_env_field(
    env_var: Option<&'static str>,
    record: &ManagedAgentRecord,
    file_value: &Option<String>,
    tiers: &InheritedConfigTiers,
) -> Option<NormalizedField> {
    let [rec_env, pers_env, glob_env, def_env] = env_var
        .map(|k| {
            env_candidates(
                k,
                &record.env_vars,
                &tiers.persona_env,
                &tiers.global_env,
                &tiers.definition_env,
            )
        })
        .unwrap_or([None, None, None, None]);

    let tiers_list: &[(Option<&str>, ConfigOrigin)] = &[
        (rec_env, ConfigOrigin::BuzzExplicit),
        (pers_env, ConfigOrigin::PersonaDefault),
        (glob_env, ConfigOrigin::GlobalDefault),
        (def_env, ConfigOrigin::HarnessDefault),
        (file_value.as_deref(), ConfigOrigin::ConfigFile),
    ];

    let (value, origin, overridden_value, overridden_origin) = resolve_with_override(tiers_list)?;

    let write_via = if let Some(key) = env_var {
        ConfigWriteMechanism::RespawnWithEnvVar {
            env_key: key.to_string(),
        }
    } else {
        ConfigWriteMechanism::ReadOnly
    };

    Some(NormalizedField {
        value,
        origin,
        write_via,
        overridden_value,
        overridden_origin,
        is_required: false,
    })
}

/// System prompt field.
///
/// Tier ordering per v3 plan: record env > persona env > global env >
/// struct record > struct persona > config file.
///
/// Env tiers sit above structured per spawn contract: `descriptor.env` is
/// written last (after the structured prompt), so env wins on collision.
/// `GlobalAgentConfig` has no structured system_prompt, so the global tier
/// is env-only. `BUZZ_ACP_SYSTEM_PROMPT` is not reserved and is therefore
/// a real global env tier.
fn build_system_prompt_field(
    record: &ManagedAgentRecord,
    file_prompt: &Option<String>,
    tiers: &InheritedConfigTiers,
) -> Option<NormalizedField> {
    const PROMPT_ENV_KEY: &str = "BUZZ_ACP_SYSTEM_PROMPT";

    let [rec_env, pers_env, glob_env, def_env] = env_candidates(
        PROMPT_ENV_KEY,
        &record.env_vars,
        &tiers.persona_env,
        &tiers.global_env,
        &tiers.definition_env,
    );

    // Structured record prompt (definition-less only; linked cleared upstream).
    let struct_record = record.system_prompt.as_deref();

    let tiers_list: &[(Option<&str>, ConfigOrigin)] = &[
        (rec_env, ConfigOrigin::BuzzExplicit),       // record env
        (pers_env, ConfigOrigin::PersonaDefault),    // persona env
        (glob_env, ConfigOrigin::GlobalDefault),     // global env
        (def_env, ConfigOrigin::HarnessDefault),     // definition env
        (struct_record, ConfigOrigin::BuzzExplicit), // struct record
        (
            tiers.persona_prompt.as_deref(),
            ConfigOrigin::PersonaDefault,
        ), // struct persona
        (file_prompt.as_deref(), ConfigOrigin::ConfigFile),
    ];

    let (value, origin, overridden_value, overridden_origin) = resolve_with_override(tiers_list)?;

    Some(NormalizedField {
        value,
        origin,
        write_via: ConfigWriteMechanism::RespawnWithEnvVar {
            env_key: PROMPT_ENV_KEY.to_string(),
        },
        overridden_value,
        overridden_origin,
        is_required: false,
    })
}

/// `(value, origin, overridden_value, overridden_origin)` — the resolved
/// winner plus the next `Some` tier it shadows, if any.
type ResolvedOverride = (
    Option<String>,
    ConfigOrigin,
    Option<String>,
    Option<ConfigOrigin>,
);

/// Picks the first `Some` value from `tiers` (highest-precedence first);
/// the overridden pair is the next `Some` tier after the winner. Returns
/// `None` when no tier has a value.
fn resolve_with_override(tiers: &[(Option<&str>, ConfigOrigin)]) -> Option<ResolvedOverride> {
    let winner_idx = tiers.iter().position(|(v, _)| v.is_some())?;
    let (value, origin) = &tiers[winner_idx];
    let value = value.map(str::to_string);
    let origin = origin.clone();

    // Overridden = the next Some after the winner.
    let overridden = tiers[winner_idx + 1..].iter().find(|(v, _)| v.is_some());
    let (overridden_value, overridden_origin) = match overridden {
        Some((v, o)) => (v.map(str::to_string), Some(o.clone())),
        None => (None, None),
    };

    Some((value, origin, overridden_value, overridden_origin))
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
#[path = "reader_tests.rs"]
mod tests;
