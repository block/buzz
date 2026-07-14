use serde::Serialize;
use tauri::{AppHandle, State};

use crate::{
    app_state::AppState,
    managed_agents::{
        config_bridge::{
            read_goose_file_config,
            reader::read_config_surface,
            types::{
                AcpConfigOptionEntry, AcpConfigOptionValue, AcpModelEntry, ConfigOrigin,
                NormalizedField, RuntimeConfigSurface, SessionConfigCache,
            },
        },
        current_instance_id, known_acp_runtime, load_managed_agents, load_personas,
        resolve_effective_prompt_model_provider, save_managed_agents, sync_managed_agent_processes,
        AgentDefinition, GlobalAgentConfig, KnownAcpRuntime, ManagedAgentRecord,
    },
};

/// Subset of the goose file config exposed to the frontend for gate evaluation.
///
/// Only the fields the dialog gate needs — not the full `RuntimeConfigSurface`.
/// The gate uses this to know which requirements are already satisfied in the
/// harness config file, so it can show "Set in goose config" rather than
/// surfacing a false missing-key marker.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeFileConfigSubset {
    /// Provider set in the harness config file, if any.
    pub provider: Option<String>,
    /// Model set in the harness config file, if any.
    pub model: Option<String>,
    /// Flat credential env keys found in the harness config file's `extra` map
    /// (e.g. `DATABRICKS_HOST`).  Only non-empty values are included.
    pub satisfied_env_keys: Vec<String>,
}

/// Resolve the config surface with persona and global default values applied.
///
/// The pipeline: resolve the linked persona's prompt/model/provider, inject
/// each into the record only where the record lacks its own value, let
/// `read_config_surface` tag those injected fields `BuzzExplicit`, then re-tag
/// exactly the injected fields to `PersonaDefault`.
///
/// Global defaults fill in when neither the record nor the linked persona
/// provides a value. They are re-tagged to `GlobalDefault` so the UI can
/// display "inherited from global defaults".
///
/// The re-tag is triple-gated — a field is re-tagged only when (a) the record
/// did not already have it (`!had_*`), (b) the surface produced the field, and
/// (c) the reader tagged it `BuzzExplicit`. A value the user set explicitly in
/// Buzz keeps `had_* == true` and is never re-tagged.
fn resolve_config_surface(
    mut record: ManagedAgentRecord,
    personas: &[AgentDefinition],
    runtime_meta: Option<&KnownAcpRuntime>,
    effective_command: &str,
    session_cache: Option<&SessionConfigCache>,
    global: &GlobalAgentConfig,
) -> RuntimeConfigSurface {
    let had_prompt =
        record.system_prompt.is_some() || record.env_vars.contains_key("BUZZ_ACP_SYSTEM_PROMPT");
    let had_model = record.model.is_some();

    let provider_env_key = runtime_meta.and_then(|m| m.provider_env_var).unwrap_or("");
    let had_provider = record.env_vars.contains_key(provider_env_key);

    let (persona_prompt, persona_model, persona_provider) = resolve_effective_prompt_model_provider(
        record.persona_id.as_deref(),
        personas,
        record.system_prompt.clone(),
        record.model.clone(),
        record.provider.clone(),
    );

    // Build the baseline the reader overrides a live model against, paired with
    // its true origin so the secondary is tagged correctly. Two sources:
    //   - persona-linked, no explicit record model: the persona model is the
    //     baseline (PersonaDefault).
    //   - genuine-explicit (record had its own model) that live-switched: the
    //     record's own model is the baseline (BuzzExplicit). Gated behind
    //     `model_overridden` so a persona edited mid-life (override flag false)
    //     never synthesizes a baseline and false-positives an override.
    // An explicit pick with no live switch has no baseline to override.
    let model_overridden = session_cache.is_some_and(|c| c.model_overridden);
    let baseline = if had_model {
        if model_overridden {
            record
                .model
                .clone()
                .map(|m| (m, ConfigOrigin::BuzzExplicit))
        } else {
            None
        }
    } else {
        // Prefer persona as baseline, fall back to global when persona has none
        // and the model was overridden mid-session (global-default agent).
        persona_model
            .clone()
            .map(|m| (m, ConfigOrigin::PersonaDefault))
            .or_else(|| {
                if model_overridden {
                    global
                        .model
                        .clone()
                        .map(|m| (m, ConfigOrigin::GlobalDefault))
                } else {
                    None
                }
            })
    };

    // Inject resolved persona values into the record where absent.
    if !had_prompt {
        if let Some(p) = persona_prompt {
            record
                .env_vars
                .insert("BUZZ_ACP_SYSTEM_PROMPT".to_string(), p);
        }
    }
    if !had_model {
        record.model = persona_model.clone();
    }
    if !had_provider && !provider_env_key.is_empty() {
        if let Some(prov) = persona_provider {
            record.env_vars.insert(provider_env_key.to_string(), prov);
        }
    }

    // Inject global defaults where neither the record nor the persona had a value.
    // Track injection so we can re-tag to GlobalDefault after the reader.
    let inject_global_model = !had_model && record.model.is_none();
    let inject_global_provider = !had_provider
        && !provider_env_key.is_empty()
        && !record.env_vars.contains_key(provider_env_key);

    if inject_global_model {
        record.model = global.model.clone();
    }
    if inject_global_provider {
        if let Some(ref gprov) = global.provider {
            record
                .env_vars
                .insert(provider_env_key.to_string(), gprov.clone());
        }
    }

    let mut surface = read_config_surface(
        &record,
        runtime_meta,
        session_cache,
        baseline.as_ref().map(|(m, o)| (m.as_str(), o.clone())),
    );

    if runtime_meta.is_some_and(|m| m.id == "buzz-agent") {
        surface.buzz_agent_mcp_servers = crate::managed_agents::effective_buzz_agent_mcp_servers(
            &record,
            personas,
            &global.mcp_servers,
            effective_command,
        )
        .unwrap_or_else(|error| {
            eprintln!("buzz-desktop: invalid persisted MCP server configuration: {error}");
            Vec::new()
        });
    }

    // Re-tag persona-sourced fields from BuzzExplicit to PersonaDefault.
    if !had_prompt {
        retag_persona_default(&mut surface.normalized.system_prompt);
    }
    if !had_model && !inject_global_model {
        retag_persona_default(&mut surface.normalized.model);
    }
    if !had_provider && !provider_env_key.is_empty() && !inject_global_provider {
        retag_persona_default(&mut surface.normalized.provider);
    }

    // Re-tag global-sourced fields from BuzzExplicit to GlobalDefault.
    if inject_global_model {
        retag_global_default(&mut surface.normalized.model);
    }
    if inject_global_provider {
        retag_global_default(&mut surface.normalized.provider);
    }

    // Re-tag persona-snapshotted model from BuzzExplicit to PersonaDefault.
    // Persona-created agents have record.model set at create time from the
    // persona snapshot — had_model is true, but the model came from the persona,
    // not an explicit user choice. Re-tag when the record model matches the
    // persona model and no live override is active. Only applies when a persona
    // is actually linked — non-persona agents with an explicit model keep BuzzExplicit.
    if had_model && !model_overridden && record.persona_id.is_some() {
        if let (Some(ref record_model), Some(ref persona_model_val)) =
            (&record.model, &persona_model)
        {
            if record_model == persona_model_val {
                retag_persona_default(&mut surface.normalized.model);
            }
        }
    }

    surface
}

/// Re-tag a field's origin from `BuzzExplicit` to `PersonaDefault`, leaving any
/// other origin untouched. No-op when the field is absent.
fn retag_persona_default(field: &mut Option<NormalizedField>) {
    if let Some(field) = field {
        if field.origin == ConfigOrigin::BuzzExplicit {
            field.origin = ConfigOrigin::PersonaDefault;
        }
    }
}

/// Get the file-layer config for a runtime — used by the Create/Edit/Persona
/// dialogs to know which requirements are already satisfied in the harness
/// config file (e.g. `~/.config/goose/config.yaml`), so they can show
/// "Set in goose config" instead of surfacing a false required-field marker.
///
/// Returns `null` when the runtime has no config file or it cannot be parsed.
/// Currently only "goose" is supported; other runtimes return `null`.
#[tauri::command]
pub async fn get_runtime_file_config(
    runtime_id: String,
) -> Result<Option<RuntimeFileConfigSubset>, String> {
    tokio::task::spawn_blocking(move || match runtime_id.as_str() {
        "goose" => {
            let cfg = read_goose_file_config()?;
            let satisfied_env_keys = cfg
                .extra
                .into_iter()
                .filter(|(_, v)| !v.is_empty())
                .map(|(k, _)| k)
                .collect();
            Some(RuntimeFileConfigSubset {
                provider: cfg.provider,
                model: cfg.model,
                satisfied_env_keys,
            })
        }
        _ => None,
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))
}

/// Return the key names of all non-empty baked build env vars.
///
/// Internal (Block) builds bake provider credentials and other env pairs into
/// the binary at compile time via `BUZZ_BUILD_AGENT_ENV`. The backend readiness
/// gate already treats these keys as satisfying their requirements (Layer 1 of
/// `resolve_effective_agent_env`). This command exposes the *key names only* —
/// never the values — so the frontend dialogs can apply the same logic and avoid
/// surfacing a spurious "Required" badge for keys that are covered by the baked
/// env.
///
/// OSS builds have no baked env, so this returns an empty list — OSS behavior
/// is unchanged.
#[tauri::command]
pub fn get_baked_build_env_keys() -> Vec<String> {
    crate::managed_agents::baked_build_env()
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, _)| k)
        .collect()
}

/// A single baked build env entry returned to the frontend.
///
/// Values are masked in Rust so unmasked secret values never cross the
/// Tauri IPC boundary. The `masked` flag lets the frontend style masked
/// rows distinctly.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BakedEnvEntry {
    pub key: String,
    /// The display value — real value for non-secret keys, `••••••` for
    /// secret keys whose names match the secret heuristic.
    pub value: String,
    /// `true` when the value was replaced by the mask placeholder.
    pub masked: bool,
}

/// Returns `true` when a baked-env key is safe to display unmasked in the UI.
///
/// This uses an explicit allowlist of keys that are known safe (non-secret).
/// Any key NOT in this set is masked — default-deny for a security surface.
///
/// Allowlist (case-insensitive):
/// - `BUZZ_AGENT_PROVIDER`, `BUZZ_AGENT_MODEL` — agent runtime selection
/// - `BUZZ_AGENT_THINKING_EFFORT` — non-secret enum (none/minimal/low/medium/high/xhigh/max)
/// - `DATABRICKS_HOST`, `DATABRICKS_MODEL` — Block non-secret defaults
fn is_safe_to_reveal(key: &str) -> bool {
    const SAFE_KEYS: &[&str] = &[
        "BUZZ_AGENT_PROVIDER",
        "BUZZ_AGENT_MODEL",
        "BUZZ_AGENT_THINKING_EFFORT",
        "DATABRICKS_HOST",
        "DATABRICKS_MODEL",
    ];
    let upper = key.to_ascii_uppercase();
    SAFE_KEYS.iter().any(|safe| upper == *safe)
}

/// Expose the baked build env to the frontend with values shown, but any
/// key not in the safe-to-reveal allowlist has its value replaced by `••••••`.
///
/// Provider and model arrive as `BUZZ_AGENT_PROVIDER` / `BUZZ_AGENT_MODEL`
/// keys in `baked_build_env()` and are included in the returned list like any
/// other key. Empty-value keys are filtered out (same as
/// `get_baked_build_env_keys`).
///
/// OSS builds return an empty list — the baked-env section is hidden entirely
/// in OSS installations.
#[tauri::command]
pub fn get_baked_build_env() -> Vec<BakedEnvEntry> {
    crate::managed_agents::baked_build_env()
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(key, value)| {
            let masked = !is_safe_to_reveal(&key);
            let display_value = if masked {
                "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}".to_string()
            } else {
                value
            };
            BakedEnvEntry {
                key,
                value: display_value,
                masked,
            }
        })
        .collect()
}

/// Re-tag a field's origin from `BuzzExplicit` to `GlobalDefault`, leaving any
/// other origin untouched. No-op when the field is absent.
fn retag_global_default(field: &mut Option<NormalizedField>) {
    if let Some(field) = field {
        if field.origin == ConfigOrigin::BuzzExplicit {
            field.origin = ConfigOrigin::GlobalDefault;
        }
    }
}

/// Get the full config surface for a managed agent.
///
/// Returns normalized + advanced config from all available tiers.
/// Pre-spawn agents show config file values with ACP tiers marked as pending.
/// Persona-sourced values are resolved by `resolve_config_surface`.
#[tauri::command]
pub async fn get_agent_config_surface(
    pubkey: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<RuntimeConfigSurface, String> {
    let record = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let mut records = load_managed_agents(&app)?;
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|e| e.to_string())?;
        let (sync_changed, exited_pubkeys) =
            sync_managed_agent_processes(&mut records, &mut runtimes, &current_instance_id(&app));
        if sync_changed {
            save_managed_agents(&app, &records)?;
        }
        for pubkey in &exited_pubkeys {
            state.clear_session_cache(pubkey);
        }
        records
            .into_iter()
            .find(|r| r.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?
    };

    let personas = load_personas(&app).unwrap_or_default();
    let effective_cmd = crate::managed_agents::record_agent_command(&record, &personas);
    let runtime_meta = known_acp_runtime(&effective_cmd);
    let session_cache = state.get_session_cache(&pubkey);
    let global = crate::managed_agents::load_global_agent_config(&app).unwrap_or_default();

    Ok(resolve_config_surface(
        record,
        &personas,
        runtime_meta,
        &effective_cmd,
        session_cache.as_ref(),
        &global,
    ))
}

/// Store a `session_config_captured` observer event payload into the session cache.
///
/// Called by the TypeScript observer relay when it decrypts a `session_config_captured`
/// event from a running agent. The payload contains raw ACP session/new fields.
#[tauri::command]
pub fn put_agent_session_config(
    pubkey: String,
    payload: serde_json::Value,
    app: AppHandle,
    state: State<'_, AppState>,
) {
    {
        let _guard = match state.managed_agents_store_lock.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        match load_managed_agents(&app) {
            Ok(records) if records.iter().any(|r| r.pubkey == pubkey) => {}
            _ => return,
        }
    }

    let config_options = parse_config_options(payload.get("configOptions"));
    let available_modes = parse_modes(&config_options, payload.get("modes"));
    let (available_models, current_model) = parse_models(payload.get("models"));
    let model_overridden = payload
        .get("modelOverridden")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let cache = SessionConfigCache {
        config_options,
        available_modes,
        available_models,
        current_model,
        model_overridden,
        goose_native_config: None,
        captured_at: crate::util::now_iso(),
    };

    state.put_session_cache(&pubkey, cache);
}

fn parse_config_options(raw: Option<&serde_json::Value>) -> Vec<AcpConfigOptionEntry> {
    let arr = match raw.and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .filter_map(|opt| {
            let config_id = opt
                .get("id")
                .or_else(|| opt.get("configId"))?
                .as_str()?
                .to_string();
            Some(AcpConfigOptionEntry {
                config_id,
                category: opt
                    .get("category")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                display_name: opt
                    .get("displayName")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                current_value: opt
                    .get("value")
                    .or_else(|| opt.get("currentValue"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                options: parse_option_values(opt.get("options")),
            })
        })
        .collect()
}

fn parse_option_values(raw: Option<&serde_json::Value>) -> Vec<AcpConfigOptionValue> {
    let arr = match raw.and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .filter_map(|o| {
            let value = o.get("value").and_then(|v| v.as_str())?.to_string();
            Some(AcpConfigOptionValue {
                value,
                display_name: o
                    .get("displayName")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            })
        })
        .collect()
}

fn parse_modes(
    config_options: &[AcpConfigOptionEntry],
    raw: Option<&serde_json::Value>,
) -> Vec<String> {
    if let Some(arr) = raw.and_then(|v| v.as_array()) {
        return arr
            .iter()
            .filter_map(|m| m.as_str().map(str::to_string))
            .collect();
    }
    // Fall back: extract mode options from configOptions with category "mode".
    config_options
        .iter()
        .filter(|o| o.category.as_deref() == Some("mode"))
        .flat_map(|o| o.options.iter().map(|v| v.value.clone()))
        .collect()
}

fn parse_models(raw: Option<&serde_json::Value>) -> (Vec<AcpModelEntry>, Option<String>) {
    let raw = match raw {
        Some(v) => v,
        None => return (Vec::new(), None),
    };

    // Object shape: { currentModelId, availableModels: [...] }
    if let Some(obj) = raw.as_object() {
        let current_model = obj
            .get("currentModelId")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let models = obj
            .get("availableModels")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| {
                        let model_id = m
                            .get("modelId")
                            .or_else(|| m.get("id"))
                            .and_then(|v| v.as_str())?
                            .to_string();
                        Some(AcpModelEntry {
                            model_id,
                            name: m.get("name").and_then(|v| v.as_str()).map(str::to_string),
                            description: m
                                .get("description")
                                .and_then(|v| v.as_str())
                                .map(str::to_string),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        return (models, current_model);
    }

    // Array shape: [{ modelId, isCurrent, ... }]
    let arr = match raw.as_array() {
        Some(a) => a,
        None => return (Vec::new(), None),
    };
    let mut current_model = None;
    let models = arr
        .iter()
        .filter_map(|m| {
            let model_id = m
                .get("modelId")
                .or_else(|| m.get("id"))
                .and_then(|v| v.as_str())?
                .to_string();
            if m.get("isCurrent")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                current_model = Some(model_id.clone());
            }
            Some(AcpModelEntry {
                model_id,
                name: m.get("name").and_then(|v| v.as_str()).map(str::to_string),
                description: m
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            })
        })
        .collect();
    (models, current_model)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::managed_agents::{BackendKind, RespondTo};

    fn goose_runtime() -> &'static KnownAcpRuntime {
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
            cli_install_commands_windows: &[],
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

    fn agent_record() -> ManagedAgentRecord {
        ManagedAgentRecord {
            mcp_servers: vec![],
            pubkey: "agent".to_string(),
            name: "Agent".to_string(),
            persona_id: Some("persona-1".to_string()),
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
            backend: BackendKind::Local,
            backend_agent_id: None,
            provider_binary_path: None,
            team_id: None,
            persona_team_dir: None,
            persona_name_in_team: None,
            created_at: "".to_string(),
            updated_at: "".to_string(),
            last_started_at: None,
            last_stopped_at: None,
            last_exit_code: None,
            last_error: None,
            last_error_code: None,
            respond_to: RespondTo::OwnerOnly,
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

    fn persona_with_model(model: &str) -> AgentDefinition {
        AgentDefinition {
            mcp_servers: vec![],
            id: "persona-1".to_string(),
            display_name: "Persona".to_string(),
            avatar_url: None,
            system_prompt: "You are a persona.".to_string(),
            runtime: None,
            model: Some(model.to_string()),
            provider: None,
            name_pool: Vec::new(),
            is_builtin: false,
            is_active: true,
            source_team: None,
            source_team_persona_slug: None,
            env_vars: BTreeMap::new(),
            respond_to: None,
            respond_to_allowlist: Vec::new(),
            parallelism: None,
            created_at: "".to_string(),
            updated_at: "".to_string(),
        }
    }

    /// A post-spawn session cache whose live model is `current_model` and whose
    /// `model_overridden` flag records whether a `SwitchModel` control signal set
    /// it (the live-switch signal).
    fn session_cache(current_model: &str, model_overridden: bool) -> SessionConfigCache {
        SessionConfigCache {
            config_options: vec![],
            available_modes: vec![],
            available_models: vec![],
            current_model: Some(current_model.to_string()),
            model_overridden,
            goose_native_config: None,
            captured_at: "".to_string(),
        }
    }

    /// A model the user set explicitly in Buzz must never be re-tagged to
    /// `PersonaDefault`, even when the linked persona also has a model.
    #[test]
    fn explicit_record_model_outranks_persona_and_keeps_buzz_explicit_origin() {
        let mut record = agent_record();
        record.model = Some("explicit-model".to_string());
        let personas = vec![persona_with_model("persona-model")];

        let surface = resolve_config_surface(
            record,
            &personas,
            Some(goose_runtime()),
            "goose",
            None,
            &Default::default(),
        );

        let model = surface.normalized.model.as_ref().expect("model resolved");
        assert_eq!(model.value.as_deref(), Some("explicit-model"));
        assert_eq!(model.origin, ConfigOrigin::BuzzExplicit);
    }

    /// Part A — pending-pick: a genuine-explicit pick X with a divergent live
    /// model Y but `model_overridden == false` (the live switch is not yet
    /// applied — a restart is pending) must keep X as the primary and must NOT
    /// surface Y as an override row. The live `acp_model` does not win. This
    /// FAILS against a let-live-acp-win variant (one that dropped the
    /// `model_overridden` gate), so it is not vacuous.
    #[test]
    fn pending_pick_keeps_explicit_x_and_does_not_surface_live_y() {
        let mut record = agent_record();
        record.persona_id = None;
        record.model = Some("model-x".to_string());
        let personas: Vec<AgentDefinition> = vec![];
        let cache = session_cache("model-y", false);

        let surface = resolve_config_surface(
            record,
            &personas,
            Some(goose_runtime()),
            "goose",
            Some(&cache),
            &Default::default(),
        );
        let model = surface.normalized.model.expect("model resolved");

        assert_eq!(model.value.as_deref(), Some("model-x"));
        assert_eq!(model.origin, ConfigOrigin::BuzzExplicit);
        assert_ne!(model.origin, ConfigOrigin::RuntimeOverride);
        assert_ne!(model.overridden_value.as_deref(), Some("model-y"));
    }

    /// W2 — genuine-explicit live switch: record.model = X, no persona,
    /// `model_overridden == true`, live model = Y. The live Y must render as the
    /// primary with a `RuntimeOverride` origin and X as the secondary tagged
    /// `BuzzExplicit` (its true source — NOT `PersonaDefault`). FAILS against the
    /// shipped no-persona early-return, which left X as primary and Y struck.
    #[test]
    fn genuine_explicit_live_switch_renders_y_over_x_buzz_explicit_secondary() {
        let mut record = agent_record();
        record.persona_id = None;
        record.model = Some("model-x".to_string());
        let personas: Vec<AgentDefinition> = vec![];
        let cache = session_cache("model-y", true);

        let surface = resolve_config_surface(
            record,
            &personas,
            Some(goose_runtime()),
            "goose",
            Some(&cache),
            &Default::default(),
        );
        let model = surface.normalized.model.expect("model resolved");

        assert_eq!(model.value.as_deref(), Some("model-y"));
        assert_eq!(model.origin, ConfigOrigin::RuntimeOverride);
        assert_eq!(model.overridden_value.as_deref(), Some("model-x"));
        assert_eq!(model.overridden_origin, Some(ConfigOrigin::BuzzExplicit));
    }

    /// Y==X collision: a genuine-explicit agent live-switches to the SAME value
    /// it already had. There is no real divergence, so the field must be a clean
    /// single value with NO secondary row. FAILS against a naive `return base`
    /// that would leak the `AcpConfigOption` row `build_model_field` populates.
    #[test]
    fn genuine_explicit_live_switch_to_same_model_yields_clean_field() {
        let mut record = agent_record();
        record.persona_id = None;
        record.model = Some("model-x".to_string());
        let personas: Vec<AgentDefinition> = vec![];
        let cache = session_cache("model-x", true);

        let surface = resolve_config_surface(
            record,
            &personas,
            Some(goose_runtime()),
            "goose",
            Some(&cache),
            &Default::default(),
        );
        let model = surface.normalized.model.expect("model resolved");

        assert_eq!(model.value.as_deref(), Some("model-x"));
        assert_eq!(model.overridden_value, None);
        assert_eq!(model.overridden_origin, None);
    }

    /// Persona parity (regression): a persona-linked agent with no explicit
    /// record model that live-switches still renders the persona model as the
    /// secondary tagged `PersonaDefault` — the typed-baseline change must NOT
    /// regress the persona arm to a different origin.
    #[test]
    fn persona_linked_live_switch_keeps_persona_default_secondary() {
        let record = agent_record();
        let personas = vec![persona_with_model("persona-model")];
        let cache = session_cache("model-y", true);

        let surface = resolve_config_surface(
            record,
            &personas,
            Some(goose_runtime()),
            "goose",
            Some(&cache),
            &Default::default(),
        );
        let model = surface.normalized.model.expect("model resolved");

        assert_eq!(model.value.as_deref(), Some("model-y"));
        assert_eq!(model.origin, ConfigOrigin::RuntimeOverride);
        assert_eq!(model.overridden_value.as_deref(), Some("persona-model"));
        assert_eq!(model.overridden_origin, Some(ConfigOrigin::PersonaDefault));
    }

    /// Fix 2 regression: a global-default-only agent (no record model, no
    /// persona model, but global has a model) that live-switches mid-session
    /// must render the global model as the secondary tagged `GlobalDefault`.
    /// Before the fix, `baseline` was `None` in the `!had_model` arm when
    /// persona has no model, so `read_config_surface` had no secondary to
    /// surface. Fails against pre-fix code where the baseline arm returned
    /// `None` when `!had_model && persona_model.is_none() && model_overridden`.
    #[test]
    fn global_default_live_switch_renders_global_model_as_secondary_global_default() {
        // Record has no model, no persona, global provides the model.
        let mut record = agent_record();
        record.persona_id = None;
        // record.model = None (set by agent_record())
        let personas: Vec<AgentDefinition> = vec![];
        let cache = session_cache("model-y", true);
        let global = crate::managed_agents::GlobalAgentConfig {
            mcp_servers: vec![],
            model: Some("global-model".to_string()),
            ..Default::default()
        };

        let surface = resolve_config_surface(
            record,
            &personas,
            Some(goose_runtime()),
            "goose",
            Some(&cache),
            &global,
        );
        let model = surface.normalized.model.expect("model resolved");

        // Live model wins as primary.
        assert_eq!(model.value.as_deref(), Some("model-y"));
        assert_eq!(model.origin, ConfigOrigin::RuntimeOverride);
        // Global model surfaces as secondary, tagged GlobalDefault.
        assert_eq!(
            model.overridden_value.as_deref(),
            Some("global-model"),
            "global model must be the override baseline secondary"
        );
        assert_eq!(
            model.overridden_origin,
            Some(ConfigOrigin::GlobalDefault),
            "override baseline origin must be GlobalDefault, not PersonaDefault or BuzzExplicit"
        );
    }

    // ── buzz_agent_mcp_servers surface tests ────────────────────────────────

    fn mcp_server(
        name: &str,
        command: &str,
        enabled: bool,
    ) -> crate::managed_agents::McpServerConfig {
        crate::managed_agents::McpServerConfig {
            name: name.to_string(),
            command: command.to_string(),
            args: vec![],
            env: vec![],
            enabled,
        }
    }

    /// The buzz-agent runtime surfaces its effective-merged MCP servers onto
    /// `RuntimeConfigSurface.buzz_agent_mcp_servers`, matching what
    /// `effective_buzz_agent_mcp_servers` (the spawn-time source of truth)
    /// would compute for the same record/persona/global layers.
    #[test]
    fn buzz_agent_runtime_surfaces_effective_merged_mcp_servers() {
        let mut record = agent_record();
        record.persona_id = None;
        record.mcp_servers = vec![mcp_server("agent-server", "agent-cmd", true)];
        let personas: Vec<AgentDefinition> = vec![];
        let global = crate::managed_agents::GlobalAgentConfig {
            mcp_servers: vec![mcp_server("global-server", "global-cmd", true)],
            ..Default::default()
        };

        let surface = resolve_config_surface(
            record,
            &personas,
            known_acp_runtime("buzz-agent"),
            "buzz-agent",
            None,
            &global,
        );

        let mut names: Vec<&str> = surface
            .buzz_agent_mcp_servers
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        names.sort_unstable();
        assert_eq!(names, vec!["agent-server", "global-server"]);
    }

    /// A disabled agent-layer entry masks a same-named global entry — the
    /// merged effective list must NOT include it. Fails against a variant
    /// that surfaces the raw record layer instead of the merged result.
    #[test]
    fn buzz_agent_disabled_override_masks_inherited_server_in_surface() {
        let mut record = agent_record();
        record.persona_id = None;
        record.mcp_servers = vec![mcp_server("shared", "", false)];
        let personas: Vec<AgentDefinition> = vec![];
        let global = crate::managed_agents::GlobalAgentConfig {
            mcp_servers: vec![mcp_server("shared", "global-cmd", true)],
            ..Default::default()
        };

        let surface = resolve_config_surface(
            record,
            &personas,
            known_acp_runtime("buzz-agent"),
            "buzz-agent",
            None,
            &global,
        );

        assert!(surface.buzz_agent_mcp_servers.is_empty());
    }

    /// Non-buzz-agent runtimes (e.g. goose) must never populate
    /// `buzz_agent_mcp_servers` — they surface their servers via `extensions`
    /// instead. Fails against a variant that populates the field unconditionally.
    #[test]
    fn non_buzz_agent_runtime_leaves_mcp_surface_empty() {
        let mut record = agent_record();
        record.mcp_servers = vec![mcp_server("agent-server", "agent-cmd", true)];
        let personas = vec![persona_with_model("persona-model")];

        let surface = resolve_config_surface(
            record,
            &personas,
            Some(goose_runtime()),
            "goose",
            None,
            &Default::default(),
        );

        assert!(surface.buzz_agent_mcp_servers.is_empty());
    }

    // ── get_baked_build_env / is_secret_key tests ──────────────────────────

    /// Build a `BakedEnvEntry` vec from a synthetic map, mirroring what
    /// `get_baked_build_env()` does. Used to test masking without relying on
    /// compile-time `option_env!` vars (OSS builds have empty `baked_build_env`).
    fn baked_env_from_map(map: &[(&str, &str)]) -> Vec<BakedEnvEntry> {
        map.iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| {
                let masked = !super::is_safe_to_reveal(k);
                BakedEnvEntry {
                    key: k.to_string(),
                    value: if masked {
                        "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}".to_string()
                    } else {
                        v.to_string()
                    },
                    masked,
                }
            })
            .collect()
    }

    #[test]
    fn baked_env_non_secret_key_shows_real_value() {
        let entries = baked_env_from_map(&[("BUZZ_AGENT_PROVIDER", "databricks_v2")]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "BUZZ_AGENT_PROVIDER");
        assert_eq!(entries[0].value, "databricks_v2");
        assert!(!entries[0].masked);
    }

    #[test]
    fn baked_env_api_key_is_masked() {
        let entries = baked_env_from_map(&[("ANTHROPIC_API_KEY", "sk-secret")]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].value, "••••••");
        assert!(entries[0].masked);
    }

    #[test]
    fn baked_env_token_key_is_masked() {
        let entries = baked_env_from_map(&[("GITHUB_TOKEN", "ghp_secret")]);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].masked);
    }

    #[test]
    fn baked_env_secret_key_is_masked() {
        let entries = baked_env_from_map(&[("MY_DB_SECRET", "s3cr3t")]);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].masked);
    }

    #[test]
    fn baked_env_password_key_is_masked() {
        let entries = baked_env_from_map(&[("DB_PASSWORD", "hunter2")]);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].masked);
    }

    #[test]
    fn baked_env_empty_value_filtered_out() {
        let entries = baked_env_from_map(&[("BUZZ_AGENT_PROVIDER", "")]);
        assert!(entries.is_empty());
    }

    #[test]
    fn baked_env_mixed_keys_correct_masking() {
        let entries = baked_env_from_map(&[
            ("BUZZ_AGENT_PROVIDER", "databricks_v2"),
            ("BUZZ_AGENT_MODEL", "goose-claude-opus-4-8"),
            ("DATABRICKS_HOST", "https://example.com"),
            ("DATABRICKS_TOKEN", "dapi-secret"),
        ]);
        assert_eq!(entries.len(), 4);

        let provider = entries
            .iter()
            .find(|e| e.key == "BUZZ_AGENT_PROVIDER")
            .unwrap();
        assert_eq!(provider.value, "databricks_v2");
        assert!(!provider.masked);

        let model = entries
            .iter()
            .find(|e| e.key == "BUZZ_AGENT_MODEL")
            .unwrap();
        assert_eq!(model.value, "goose-claude-opus-4-8");
        assert!(!model.masked);

        let host = entries.iter().find(|e| e.key == "DATABRICKS_HOST").unwrap();
        assert_eq!(host.value, "https://example.com");
        assert!(!host.masked);

        let token = entries
            .iter()
            .find(|e| e.key == "DATABRICKS_TOKEN")
            .unwrap();
        assert_eq!(token.value, "••••••");
        assert!(token.masked);
    }

    #[test]
    fn baked_env_thinking_effort_is_unmasked() {
        // BUZZ_AGENT_THINKING_EFFORT is a non-secret enum — must not be masked.
        let entries = baked_env_from_map(&[("BUZZ_AGENT_THINKING_EFFORT", "medium")]);
        assert_eq!(entries.len(), 1);
        let effort = entries
            .iter()
            .find(|e| e.key == "BUZZ_AGENT_THINKING_EFFORT")
            .unwrap();
        assert_eq!(effort.value, "medium");
        assert!(!effort.masked);
    }

    #[test]
    fn baked_env_allowlist_is_case_insensitive() {
        // Known-safe keys — case-insensitive match must allow them.
        assert!(super::is_safe_to_reveal("buzz_agent_provider"));
        assert!(super::is_safe_to_reveal("BUZZ_AGENT_PROVIDER"));
        assert!(super::is_safe_to_reveal("buzz_agent_model"));
        assert!(super::is_safe_to_reveal("BUZZ_AGENT_MODEL"));
        assert!(super::is_safe_to_reveal("buzz_agent_thinking_effort"));
        assert!(super::is_safe_to_reveal("BUZZ_AGENT_THINKING_EFFORT"));
        assert!(super::is_safe_to_reveal("databricks_host"));
        assert!(super::is_safe_to_reveal("DATABRICKS_HOST"));
        assert!(super::is_safe_to_reveal("databricks_model"));
        assert!(super::is_safe_to_reveal("DATABRICKS_MODEL"));
        // Keys NOT in the allowlist — masked regardless of naming pattern.
        assert!(!super::is_safe_to_reveal("my_api_key"));
        assert!(!super::is_safe_to_reveal("GITHUB_TOKEN"));
        assert!(!super::is_safe_to_reveal("DB_SECRET"));
        assert!(!super::is_safe_to_reveal("DB_PASSWORD"));
        // Bare names that old heuristic (contains("_TOKEN") etc.) would have missed.
        assert!(!super::is_safe_to_reveal("APIKEY"));
        assert!(!super::is_safe_to_reveal("TOKEN"));
        assert!(!super::is_safe_to_reveal("SECRET"));
        assert!(!super::is_safe_to_reveal("PASSWORD"));
        assert!(!super::is_safe_to_reveal("PRIVATE_KEY"));
        // Unknown key → masked by default.
        assert!(!super::is_safe_to_reveal("SOME_UNKNOWN_KEY"));
    }
}
