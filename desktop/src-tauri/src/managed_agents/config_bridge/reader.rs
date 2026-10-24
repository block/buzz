use crate::managed_agents::discovery::EffortNormalization;
use crate::managed_agents::discovery::KnownAcpRuntime;
use crate::managed_agents::types::ManagedAgentRecord;

use super::effort::effort_tier_alias;
use super::types::*;
use super::LEGACY_THINKING_EFFORT_KEY;

/// Build the full config surface for an agent, merging all tiers.
///
/// Inherited values flow through `tiers` — a sanitized snapshot of the
/// persona and global tiers assembled at the command boundary. Each field
/// builder constructs its own candidate list and resolves via
/// `resolve_with_override`.
///
/// `claude_config_dir` — when `Some`, the panel reads claude `settings.json`
/// and `.claude.json` from that directory (the agent's effective
/// `CLAUDE_CONFIG_DIR`) instead of `~/.claude/`. Ignored for non-claude runtimes.
pub(crate) fn read_config_surface(
    record: &ManagedAgentRecord,
    runtime_meta: Option<&KnownAcpRuntime>,
    session_cache: Option<&SessionConfigCache>,
    tiers: &InheritedConfigTiers,
    claude_config_dir: Option<&std::path::Path>,
) -> RuntimeConfigSurface {
    let is_pre_spawn = session_cache.is_none();

    // Tier 2b: config file values.
    let (file_config, file_was_read) = runtime_meta
        .map(|m| m.id)
        .and_then(|id| match id {
            "goose" => super::goose::read_config_file().map(|c| (c, true)),
            "claude" => super::claude::read_config_file(claude_config_dir).map(|c| (c, true)),
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
    let effort_norm = runtime_meta.and_then(|m| m.effort_normalization);
    let effort_accepted = runtime_meta.and_then(|m| m.effort_accepted_values);
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

    // B5: the adapter-advertised effort control, selected ONCE by its category.
    // The adapter defines it as category `thought_level` with its own config id
    // (Claude Code emits `id="effort"`); reading by the literal category `effort`
    // would miss it entirely. The running value, the write config id, and the
    // picker options all derive from this single entry.
    let effort_option = session_cache.and_then(find_effort_option);
    let acp_effort = effort_option.and_then(|o| o.current_value.clone());

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
            effort_option.map(|o| o.config_id.as_str()),
            thinking_env_var,
            effort_norm,
            effort_accepted,
            is_pre_spawn,
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
    let mut normalized_env_keys: Vec<&str> = [
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

    // Hide the legacy effort key from advanced only when it actually wins the
    // record tier: native and canonical column are absent/invalid, then legacy
    // normalizes. Otherwise `build_thinking_field` represents another winner
    // and the legacy key stays editable in Advanced.
    let record_legacy_consumed = thinking_env_var
        .zip(effort_norm)
        .is_some_and(|(native, norm)| {
            native != LEGACY_THINKING_EFFORT_KEY
                && super::effort::get_ci(&record.env_vars, native)
                    .and_then(|v| norm.normalize_str(v))
                    .is_none()
                && record
                    .effort_level
                    .as_deref()
                    .and_then(|v| norm.normalize_str(v))
                    .is_none()
                && super::effort::get_ci(&record.env_vars, LEGACY_THINKING_EFFORT_KEY)
                    .and_then(|v| norm.normalize_str(v))
                    .is_some()
        });
    if record_legacy_consumed {
        normalized_env_keys.push(LEGACY_THINKING_EFFORT_KEY);
    }

    // Tier 2a: remaining env vars not covered by normalized fields. Matching is
    // ASCII-case-insensitive so a mixed-case managed key (e.g. Windows
    // `goose_thinking_effort`) the launch projection already consumed is hidden
    // from Advanced rather than shown as a spurious editable extra.
    let mut advanced = advanced;
    for (k, v) in &record.env_vars {
        if normalized_env_keys
            .iter()
            .any(|nk| nk.eq_ignore_ascii_case(k))
        {
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

    let config_file_path = config_file_path_for_runtime(runtime_meta, claude_config_dir);
    let mcp_config_file_path =
        runtime_meta.and_then(|m| mcp_config_file_path_for_runtime(m, claude_config_dir));
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

    // B5: the adapter-advertised effort control, discovered once above. The UI
    // uses `effort_config_id` to send `set_config_option` and renders
    // `effort_options` instead of hardcoded values (never hardcoded here).
    let effort_config_id = effort_option.map(|o| o.config_id.clone());
    let effort_options = effort_option.map(|o| o.options.clone()).unwrap_or_default();

    RuntimeConfigSurface {
        runtime_id: runtime_meta.map(|m| m.id.to_string()),
        runtime_label: runtime_meta.map(|m| m.label.to_string()),
        is_pre_spawn,
        normalized,
        advanced,
        extensions,
        sources,
        claude_config_dir_custom: claude_config_dir.is_some(),
        effort_config_id,
        effort_options,
    }
}

/// Resolve the reported `settings.json` path. #3493: for a claude agent with a
/// custom `CLAUDE_CONFIG_DIR`, the reader reads `<custom>/settings.json`, so the
/// reported path must point there — not the static `~/.claude/settings.json`
/// from the runtime metadata. All other runtimes (and claude with no custom
/// dir) use the static metadata path.
fn config_file_path_for_runtime(
    runtime_meta: Option<&KnownAcpRuntime>,
    claude_config_dir: Option<&std::path::Path>,
) -> Option<String> {
    let runtime = runtime_meta?;
    if runtime.id == "claude" {
        if let Some(dir) = claude_config_dir {
            return Some(dir.join("settings.json").to_string_lossy().into_owned());
        }
    }
    runtime.config_file_path.map(resolve_tilde)
}

fn mcp_config_file_path_for_runtime(
    runtime: &KnownAcpRuntime,
    claude_config_dir: Option<&std::path::Path>,
) -> Option<String> {
    match runtime.id {
        "goose" => {
            super::goose::goose_config_path().map(|path| path.to_string_lossy().into_owned())
        }
        // #3493: the claude 2.1.x binary resolves .claude.json as
        // join(CLAUDE_CONFIG_DIR || homedir(), ".claude.json"), so the MCP
        // config file moves with a user-set CLAUDE_CONFIG_DIR.
        "claude" => Some(
            claude_config_dir
                .map(|d| d.join(".claude.json"))
                .unwrap_or_else(|| {
                    dirs::home_dir()
                        .map(|h| h.join(".claude.json"))
                        .unwrap_or_default()
                })
                .to_string_lossy()
                .into_owned(),
        ),
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
    effort_config_id: Option<&str>,
    thinking_env_var: Option<&str>,
    effort_norm: Option<&'static EffortNormalization>,
    effort_accepted: Option<&'static [&'static str]>,
    is_pre_spawn: bool,
    tiers: &InheritedConfigTiers,
) -> Option<NormalizedField> {
    // Tier ordering (mirrors the launch projection in `config_bridge::effort`,
    // plus the two reader-only tiers the projection has no input for — live ACP
    // and the on-disk config file):
    //   record native > canonical column > record legacy > ACP >
    //   persona > global > definition > config file.
    //
    // Every candidate is normalized through the runtime's declared contract
    // (`effort_norm`) before validity, precedence, override tracking, and the B
    // same-value collapse — the SAME normalizer the launch projection applies —
    // so the panel and the next spawn resolve one effective value AND authority.
    // For contract runtimes an invalid value (e.g. Goose `minimal`) normalizes
    // to `None` and is skipped as absent so a lower tier can win; aliases
    // (`none`→`off`, `xhigh`→`max`, case-fold) canonicalize. Contract-less
    // runtimes (buzz-agent, Claude/Codex column) pass raw.
    let norm = |raw: &str| -> Option<String> {
        super::effort::normalize_effort(effort_norm, effort_accepted, raw)
    };

    // Record tiers, split exactly as the projection resolves them: native env
    // strictly above the canonical column, legacy env strictly below it.
    let rec_native = thinking_env_var
        .and_then(|k| super::effort::get_ci(&record.env_vars, k))
        .and_then(|v| norm(v));
    let column = record.effort_level.as_deref().and_then(&norm);
    let rec_legacy = thinking_env_var
        .filter(|k| *k != LEGACY_THINKING_EFFORT_KEY)
        .and_then(|_| super::effort::get_ci(&record.env_vars, LEGACY_THINKING_EFFORT_KEY))
        .and_then(|v| norm(v));

    // Inherited env tiers: persona resolves native-then-legacy; global and
    // definition are native-only (legacy alias excluded), matching the launch
    // projection's per-tier alias policy.
    let pers = thinking_env_var.and_then(|k| effort_tier_alias(&tiers.persona_env, k, norm, true));
    let glob = thinking_env_var.and_then(|k| effort_tier_alias(&tiers.global_env, k, norm, false));
    let def =
        thinking_env_var.and_then(|k| effort_tier_alias(&tiers.definition_env, k, norm, false));
    let file = file_effort.as_deref().and_then(&norm);

    // Live ACP value: normalized through the runtime CONTRACT only, never the
    // persisted `effort_accepted` vocabulary. The ACP running value comes from
    // the session's own config-option namespace (e.g. buzz-agent reports
    // `default` for its live thinking-level option) — it is a descriptive
    // "currently running" fact, never emitted to a spawn, so the
    // destination-vocabulary gate that guards the writable tiers must not skip
    // it. Goose still canonicalizes (its ACP option values ARE effort values);
    // contract-less runtimes pass raw. The matched `config_id` is preserved for
    // `write_via` regardless of value validity.
    let acp_norm = acp_effort
        .as_deref()
        .and_then(|v| super::effort::normalize_effort(effort_norm, None, v));

    // B same-value collapse: when NO record-level authority exists and the live
    // ACP value exactly equals what inheritance would already resolve to, drop
    // ACP so the panel shows the true baseline origin ("Global default") rather
    // than a spurious "Runtime override (this session only)" — the session is
    // almost certainly echoing what spawn injected. When a record tier is
    // present it wins over ACP anyway, so ACP stays only for override tracking.
    let record_present = rec_native.is_some() || column.is_some() || rec_legacy.is_some();
    let baseline_first = [
        pers.as_deref(),
        glob.as_deref(),
        def.as_deref(),
        file.as_deref(),
    ]
    .into_iter()
    .flatten()
    .next();
    let acp_for_list = match (record_present, acp_norm.as_deref(), baseline_first) {
        (false, Some(a), Some(b)) if a == b => None,
        _ => acp_norm.as_deref(),
    };

    let tiers_list: &[(Option<&str>, ConfigOrigin)] = &[
        (rec_native.as_deref(), ConfigOrigin::BuzzExplicit),
        (column.as_deref(), ConfigOrigin::BuzzExplicit),
        (rec_legacy.as_deref(), ConfigOrigin::BuzzExplicit),
        (acp_for_list, ConfigOrigin::AcpConfigOption),
        (pers.as_deref(), ConfigOrigin::PersonaDefault),
        (glob.as_deref(), ConfigOrigin::GlobalDefault),
        (def.as_deref(), ConfigOrigin::HarnessDefault),
        (file.as_deref(), ConfigOrigin::ConfigFile),
    ];
    let (value, origin, overridden_value, overridden_origin) = resolve_with_override(tiers_list)?;

    let write_via = match (is_pre_spawn, effort_config_id, thinking_env_var) {
        (false, Some(config_id), _) => ConfigWriteMechanism::AcpSetConfigOption {
            config_id: config_id.to_string(),
        },
        (_, _, Some(env_key)) => ConfigWriteMechanism::RespawnWithEnvVar {
            env_key: env_key.to_string(),
        },
        _ => ConfigWriteMechanism::ReadOnly,
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

/// Selects the adapter-advertised effort control from the session cache.
///
/// The adapter emits effort under category `thought_level` with its own
/// config id (Claude Code uses `id="effort"`). Selecting by category — not by
/// a hardcoded id — is what lets the running value, the write config id, and
/// the picker options all derive from one entry.
///
/// `thought_level` is preferred; the legacy invented category `effort` is a
/// fallback for old test fixtures and pre-canonical adapters. The fallback
/// fires only when `thought_level` is entirely absent — an advertised-but-unset
/// `thought_level` entry is still returned (its `current_value` is `None`), so
/// the reader never flips write-routing to the legacy `effort` config id.
fn find_effort_option(cache: &SessionConfigCache) -> Option<&AcpConfigOptionEntry> {
    let by_category = |category: &str| {
        cache
            .config_options
            .iter()
            .find(|o| o.category.as_deref() == Some(category))
    };
    by_category("thought_level").or_else(|| by_category("effort"))
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
