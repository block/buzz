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
        .cloned()
        .or_else(|| record.provider.clone()); // structured provider field as fallback

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
    // For unstable/switchable agents, current_model comes from the `models`
    // field. For stable agents that only report model via configOptions
    // (category="model", current_value), fall back to find_config_option_value
    // so their current model is surfaced in the panel.
    let acp_model = session_cache.and_then(|c| {
        c.current_model
            .clone()
            .or_else(|| find_config_option_value(c, "model"))
    });
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
                required_fields.contains(&"model"),
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
            required_fields.contains(&"provider"),
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
        max_output_tokens: build_numeric_env_field(
            max_tokens_env_var,
            &record.env_vars,
            &file_config.max_output_tokens,
        ),
        context_limit: build_numeric_env_field(
            context_limit_env_var,
            &record.env_vars,
            &file_config.context_limit,
        ),
        system_prompt: build_system_prompt_field(
            &record
                .system_prompt
                .clone()
                .or_else(|| record.env_vars.get("BUZZ_ACP_SYSTEM_PROMPT").cloned()),
            &file_config.system_prompt,
        ),
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

    // Collect the env var keys already covered by normalized fields so we don't double-surface them.
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
    is_required: bool,
) -> NormalizedField {
    // Precedence: Buzz-explicit > ACP current > config file
    let (value, origin) = if let Some(ref m) = record_model {
        (Some(m.clone()), ConfigOrigin::BuzzExplicit)
    } else if let Some(ref m) = acp_model {
        (Some(m.clone()), ConfigOrigin::AcpConfigOption)
    } else if let Some(ref m) = file_model {
        (Some(m.clone()), ConfigOrigin::ConfigFile)
    } else {
        // No value from any tier. EnvVar is the sentinel origin for "no value
        // resolved" — there is no dedicated None-origin variant. The panel
        // renders this as an empty/absent field.
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
        write_via,
        overridden_value,
        overridden_origin,
        is_required,
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
    is_required: bool,
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

    let tiers: &[(Option<&str>, ConfigOrigin)] = &[
        (record_provider.as_deref(), ConfigOrigin::BuzzExplicit),
        (file_provider.as_deref(), ConfigOrigin::ConfigFile),
    ];
    let (value, origin, overridden_value, overridden_origin) = match resolve_with_override(tiers) {
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

fn build_thinking_field(
    record_effort: &Option<String>,
    file_effort: &Option<String>,
    acp_effort: &Option<String>,
    thinking_env_var: Option<&str>,
    is_pre_spawn: bool,
    session_cache: Option<&SessionConfigCache>,
) -> Option<NormalizedField> {
    let tiers: &[(Option<&str>, ConfigOrigin)] = &[
        (record_effort.as_deref(), ConfigOrigin::BuzzExplicit),
        (acp_effort.as_deref(), ConfigOrigin::AcpConfigOption),
        (file_effort.as_deref(), ConfigOrigin::ConfigFile),
    ];
    let (value, origin, overridden_value, overridden_origin) = resolve_with_override(tiers)?;

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

/// Numeric fields (max_output_tokens, context_limit) — env-var tier wins over
/// config-file tier. When an env var key is given and present in the record's
/// env_vars map the field is BuzzExplicit + RespawnWithEnvVar; otherwise if the
/// config file supplied a value it is ConfigFile + ReadOnly; otherwise None.
fn build_numeric_env_field(
    env_var: Option<&'static str>,
    record_env: &std::collections::BTreeMap<String, String>,
    file_value: &Option<String>,
) -> Option<NormalizedField> {
    if let Some(key) = env_var {
        if let Some(v) = record_env.get(key) {
            return Some(NormalizedField {
                value: Some(v.clone()),
                origin: ConfigOrigin::BuzzExplicit,
                write_via: ConfigWriteMechanism::RespawnWithEnvVar {
                    env_key: key.to_string(),
                },
                overridden_value: file_value.clone(),
                overridden_origin: file_value.as_ref().map(|_| ConfigOrigin::ConfigFile),
                is_required: false,
            });
        }
    }
    file_value.as_ref().map(|v| NormalizedField {
        value: Some(v.clone()),
        origin: ConfigOrigin::ConfigFile,
        write_via: ConfigWriteMechanism::ReadOnly,
        overridden_value: None,
        overridden_origin: None,
        is_required: false,
    })
}

/// Record/env prompt wins (BuzzExplicit, respawnable); a config-file prompt it
/// shadows is reported as the overridden secondary. A config-file-only prompt
/// — no record/env value to shadow it — is surfaced directly (read-only)
/// instead of being dropped: a prompt that drives the agent should always be
/// visible somewhere in the panel.
fn build_system_prompt_field(
    record_prompt: &Option<String>,
    file_prompt: &Option<String>,
) -> Option<NormalizedField> {
    if let Some(v) = record_prompt {
        return Some(NormalizedField {
            value: Some(v.clone()),
            origin: ConfigOrigin::BuzzExplicit,
            write_via: ConfigWriteMechanism::RespawnWithEnvVar {
                env_key: "BUZZ_ACP_SYSTEM_PROMPT".to_string(),
            },
            overridden_value: file_prompt.clone(),
            overridden_origin: file_prompt.as_ref().map(|_| ConfigOrigin::ConfigFile),
            is_required: false,
        });
    }

    file_prompt.as_ref().map(|v| NormalizedField {
        value: Some(v.clone()),
        origin: ConfigOrigin::ConfigFile,
        write_via: ConfigWriteMechanism::ReadOnly,
        overridden_value: None,
        overridden_origin: None,
        is_required: false,
    })
}

/// Picks the first `Some` value from `tiers` (highest-precedence first) and
/// returns `(value, origin, overridden_value, overridden_origin)` where the
/// overridden pair is the next `Some` tier after the winner. Returns `None`
/// when no tier has a value.
fn resolve_with_override(
    tiers: &[(Option<&str>, ConfigOrigin)],
) -> Option<(
    Option<String>,
    ConfigOrigin,
    Option<String>,
    Option<ConfigOrigin>,
)> {
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
