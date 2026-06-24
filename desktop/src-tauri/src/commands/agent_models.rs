use std::collections::HashSet;

use nostr::Keys;
use tauri::{AppHandle, State};

use crate::{
    app_state::AppState,
    managed_agents::{
        build_managed_agent_summary, default_agent_workdir, find_managed_agent_mut,
        known_acp_runtime, load_managed_agents, load_personas, managed_agent_avatar_url,
        missing_command_message, normalize_agent_args, resolve_command,
        resolve_effective_prompt_model_provider, save_managed_agents, sync_managed_agent_processes,
        try_regenerate_nest, AgentModelInfo, AgentModelsResponse, UpdateManagedAgentRequest,
        UpdateManagedAgentResponse,
    },
    relay::{relay_ws_url_with_override, sync_managed_agent_profile},
    util::now_iso,
};

/// Query available models from an agent via `buzz-acp models --json`.
///
/// Spawns a short-lived subprocess (no relay connection needed). The subprocess
/// starts the agent, queries its model catalog, and exits. ~2-5s total.
#[tauri::command]
pub async fn get_agent_models(
    pubkey: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AgentModelsResponse, String> {
    let (resolved_acp, agent_command, agent_args, persisted_model, merged_env) = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let mut records = load_managed_agents(&app)?;
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|e| e.to_string())?;
        if sync_managed_agent_processes(&mut records, &mut runtimes) {
            save_managed_agents(&app, &records)?;
        }

        let record = records
            .iter()
            .find(|r| r.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?;

        let resolved = resolve_command(&record.acp_command)
            .ok_or_else(|| missing_command_message(&record.acp_command, "ACP harness command"))?;

        // Resolve the effective harness from the linked persona (mirrors spawn),
        // so model discovery runs against the persona's current harness, not the
        // frozen record snapshot. An explicit per-agent override wins.
        let personas = load_personas(&app).unwrap_or_default();
        let effective_command = crate::managed_agents::effective_agent_command(
            record.persona_id.as_deref(),
            &personas,
            record.agent_command_override.as_deref(),
        );

        let args = normalize_agent_args(&effective_command, record.agent_args.clone());

        let resolved_agent = resolve_command(&effective_command)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| effective_command.clone());

        // Same env layering as runtime spawn: persona env < agent env.
        // Model discovery needs the user's credentials. Fail closed on
        // persona-resolution errors so a corrupt personas.json doesn't
        // produce a model list as if the persona had no credentials.
        let persona_env =
            crate::managed_agents::resolve_persona_env(&app, record.persona_id.as_deref())?;
        let env = crate::managed_agents::merged_user_env(&persona_env, &record.env_vars);

        // Resolve the effective model from the linked persona so the ModelPicker
        // dropdown shows the current persona model as selected.
        let (_prompt, effective_model, _provider) = resolve_effective_prompt_model_provider(
            record.persona_id.as_deref(),
            &personas,
            record.system_prompt.clone(),
            record.model.clone(),
        );

        (resolved, resolved_agent, args, effective_model, env)
    }; // store lock released — subprocess runs without holding the lock

    // Clone the env map for redaction below — `merged_env` is moved
    // into the spawn_blocking closure and we still need the values to
    // scrub any user-supplied secrets that the child surfaces in stderr.
    let env_for_redaction = merged_env.clone();

    // Use spawn_blocking because the desktop Tauri crate doesn't enable
    // tokio's `process` feature. std::process::Command is synchronous
    // but fine for a short-lived subprocess (~2-5s).
    let output = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new(&resolved_acp);
        if let Some(home) = default_agent_workdir() {
            cmd.current_dir(home);
        }
        if let Some(ref path) = crate::managed_agents::login_shell_path() {
            cmd.env("PATH", path);
        }
        cmd.arg("models")
            .arg("--json")
            .env("BUZZ_ACP_AGENT_COMMAND", &agent_command)
            .env("BUZZ_ACP_AGENT_ARGS", agent_args.join(","));
        if let Some(meta) = known_acp_runtime(&agent_command) {
            for (key, value) in meta.default_env {
                if std::env::var(key).is_err() {
                    cmd.env(key, value);
                }
            }
        }
        // User env layering — written LAST so it overrides any Buzz-set env above.
        for (k, v) in &merged_env {
            cmd.env(k, v);
        }
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .map_err(|e| format!("failed to spawn buzz-acp models: {e}"))
    })
    .await
    .map_err(|e| format!("model discovery task failed: {e}"))?
    .map_err(|e: String| e)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Scrub any user-supplied env values before surfacing stderr to
        // the frontend — persona/agent env_vars may carry API keys that
        // a failing child process echoed back.
        let stderr_redacted =
            crate::managed_agents::redact_env_values_in(stderr.as_ref(), &env_for_redaction);
        return Err(format!(
            "buzz-acp models failed (exit {}): {stderr_redacted}",
            output.status.code().unwrap_or(-1)
        ));
    }

    let raw: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("failed to parse model JSON: {e}"))?;

    Ok(normalize_agent_models(&raw, persisted_model))
}

/// Update mutable fields on an existing managed agent record.
///
/// Does NOT auto-restart the agent. Runtime config changes (system prompt,
/// parallelism, commands, toolsets) take effect on the next agent spawn.
/// Name changes are synced to the relay immediately via a kind:0 re-publish.
#[tauri::command]
pub async fn update_managed_agent(
    input: UpdateManagedAgentRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<UpdateManagedAgentResponse, String> {
    // Phase 1: local save (synchronous, under lock)
    let (summary, sync_params) = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let mut records = load_managed_agents(&app)?;
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|e| e.to_string())?;
        sync_managed_agent_processes(&mut records, &mut runtimes);

        let record = find_managed_agent_mut(&mut records, &input.pubkey)?;

        let mut name_changed = false;
        if let Some(name_update) = input.name {
            let trimmed = name_update.trim().to_string();
            if !trimmed.is_empty() && trimmed != record.name {
                record.name = trimmed;
                name_changed = true;
            }
        }
        if let Some(model_update) = input.model {
            record.model = model_update;
        }
        if let Some(prompt_update) = input.system_prompt {
            record.system_prompt = prompt_update;
        }
        if let Some(toolsets_update) = input.mcp_toolsets {
            record.mcp_toolsets = toolsets_update
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string);
        }
        if let Some(parallelism) = input.parallelism {
            record.parallelism = parallelism;
        }
        if let Some(turn_timeout_seconds) = input.turn_timeout_seconds {
            record.turn_timeout_seconds = turn_timeout_seconds;
        }
        // Store the relay override exactly as supplied (trimmed). An explicit
        // value pins the agent; empty falls back to the workspace relay at
        // read-time. A name-only edit (relay_url == None) leaves the pin intact.
        if let Some(relay_url) = input.relay_url {
            record.relay_url = relay_url.trim().to_string();
        }
        if let Some(acp_command) = input.acp_command {
            record.acp_command = acp_command;
        }
        // Harness edit: the persona's runtime is authoritative, so we persist an
        // explicit `agent_command_override` ONLY when the user picks a command
        // that diverges from the persona. An empty/whitespace value (the
        // "Inherit from persona" sentinel) clears the pin back to `None`. A
        // name-only edit (`agent_command == None`) leaves the pin intact.
        if let Some(agent_command) = input.agent_command {
            let personas = load_personas(&app).unwrap_or_default();
            record.agent_command_override = crate::managed_agents::divergent_agent_command_override(
                record.persona_id.as_deref(),
                &personas,
                Some(&agent_command),
            );
        }
        if let Some(agent_args) = input.agent_args {
            record.agent_args = agent_args;
        }
        if let Some(mcp_command) = input.mcp_command {
            record.mcp_command = mcp_command;
        }
        if let Some(env_vars) = input.env_vars {
            crate::managed_agents::validate_user_env_keys(&env_vars)?;
            record.env_vars = env_vars;
        }

        // Inbound author gate: merge patch onto current values, then validate
        // the merged state. This lets a single update switch to Allowlist AND
        // supply pubkeys atomically.
        let prospective_mode = input.respond_to.unwrap_or(record.respond_to);
        let prospective_allowlist = match input.respond_to_allowlist.as_ref() {
            Some(list) => crate::managed_agents::validate_respond_to_allowlist(list)?,
            None => record.respond_to_allowlist.clone(),
        };
        if prospective_mode == crate::managed_agents::RespondTo::Allowlist
            && prospective_allowlist.is_empty()
        {
            return Err(
                "respond-to mode 'allowlist' requires at least one pubkey in the allowlist"
                    .to_string(),
            );
        }
        record.respond_to = prospective_mode;
        // Preserve the persisted allowlist across mode toggles — only replace
        // when the caller explicitly supplied a new list.
        if input.respond_to_allowlist.is_some() {
            record.respond_to_allowlist = prospective_allowlist;
        }

        record.updated_at = now_iso();

        save_managed_agents(&app, &records)?;

        let record = records
            .iter()
            .find(|r| r.pubkey == input.pubkey)
            .ok_or_else(|| format!("agent {} not found", input.pubkey))?;

        let sync_params = if name_changed {
            let agent_keys = Keys::parse(&record.private_key_nsec)
                .map_err(|e| format!("failed to parse agent keys: {e}"))?;
            // Re-publish the renamed profile to the agent's effective relay:
            // an explicit per-agent relay wins; empty falls back to workspace.
            let relay_url = crate::relay::effective_agent_relay_url(
                &record.relay_url,
                &relay_ws_url_with_override(&state),
            );
            let display_name = record.name.clone();
            // Avatar fallback derives from the EFFECTIVE harness (persona-wins),
            // not the frozen snapshot, so an inherited harness picks the right
            // default avatar.
            let personas = load_personas(&app).unwrap_or_default();
            let effective_command = crate::managed_agents::effective_agent_command(
                record.persona_id.as_deref(),
                &personas,
                record.agent_command_override.as_deref(),
            );
            let avatar_url = record
                .avatar_url
                .clone()
                .or_else(|| managed_agent_avatar_url(&effective_command));
            let auth_tag = record.auth_tag.clone();
            Some((agent_keys, relay_url, display_name, avatar_url, auth_tag))
        } else {
            None
        };

        let summary = {
            let personas = load_personas(&app).unwrap_or_default();
            build_managed_agent_summary(&app, record, &runtimes, &personas)?
        };
        (summary, sync_params)
    }; // lock dropped here

    try_regenerate_nest(&app);

    // Phase 2: relay profile sync (async, best-effort, outside lock)
    let profile_sync_error =
        if let Some((agent_keys, relay_url, display_name, avatar_url, auth_tag)) = sync_params {
            match sync_managed_agent_profile(
                &state,
                &relay_url,
                &agent_keys,
                &display_name,
                avatar_url.as_deref(),
                auth_tag.as_deref(),
            )
            .await
            {
                Ok(()) => None,
                Err(e) => {
                    eprintln!("buzz-desktop: relay profile sync failed after rename: {e}");
                    Some(e)
                }
            }
        } else {
            None
        };

    Ok(UpdateManagedAgentResponse {
        agent: summary,
        profile_sync_error,
    })
}

// ── Model normalization ───────────────────────────────────────────────────────

/// Normalize raw `buzz-acp models --json` output into a typed DTO for the frontend.
///
/// Merges models from both ACP paths (stable configOptions + unstable SessionModelState),
/// deduplicates by ID (stable takes precedence), and returns a unified list.
fn normalize_agent_models(
    raw: &serde_json::Value,
    persisted_model: Option<String>,
) -> AgentModelsResponse {
    let agent_name = raw["agent"]["name"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    let agent_version = raw["agent"]["version"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    let mut models: Vec<AgentModelInfo> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();

    // 1. Stable configOptions (preferred). Only entries with category "model"
    //    are model options — the CLI pre-filters, but we're defensive here.
    if let Some(config_options) = raw["stable"]["configOptions"].as_array() {
        for opt in config_options {
            if opt.get("category").and_then(|c| c.as_str()) != Some("model") {
                continue;
            }
            if let Some(options) = opt.get("options").and_then(|v| v.as_array()) {
                for o in options {
                    if let Some(value) = o.get("value").and_then(|v| v.as_str()) {
                        if seen_ids.insert(value.to_string()) {
                            models.push(AgentModelInfo {
                                id: value.to_string(),
                                name: o
                                    .get("displayName")
                                    .and_then(|v| v.as_str())
                                    .map(str::to_string),
                                description: None,
                            });
                        }
                    }
                }
            }
        }
    }

    // 2. Unstable availableModels (fallback — skip duplicates from stable).
    let mut agent_default_model: Option<String> = None;
    if let Some(unstable) = raw.get("unstable") {
        agent_default_model = unstable["currentModelId"].as_str().map(str::to_string);
        if let Some(available) = unstable["availableModels"].as_array() {
            for m in available {
                if let Some(id) = m.get("modelId").and_then(|v| v.as_str()) {
                    if seen_ids.insert(id.to_string()) {
                        models.push(AgentModelInfo {
                            id: id.to_string(),
                            name: m.get("name").and_then(|v| v.as_str()).map(str::to_string),
                            description: m
                                .get("description")
                                .and_then(|v| v.as_str())
                                .map(str::to_string),
                        });
                    }
                }
            }
        }
    }

    let supports_switching = !models.is_empty();

    AgentModelsResponse {
        agent_name,
        agent_version,
        models,
        agent_default_model,
        selected_model: persisted_model,
        supports_switching,
    }
}
