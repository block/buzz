use tauri::{AppHandle, State};

use super::agent_model_process::run_acp_helper_subprocess;
use super::agent_models::{agent_model_discovery_config, AgentModelDiscoveryConfig};

use crate::{
    app_state::AppState,
    managed_agents::{
        discovery_env_with_baked_floor, known_acp_runtime, load_global_agent_config,
        load_managed_agents, load_personas, missing_command_message, resolve_command,
        runtime_metadata_env_vars, user_facing_harness_error,
    },
};

/// Run one bounded prompt through a managed agent and return its reply text.
///
/// Spawns a short-lived `buzz-acp complete` subprocess against the agent's
/// effective harness config (same descriptor resolution as spawn/model
/// discovery). No relay connection — the exchange never appears in any
/// channel. Powers Developer Mode channel naming.
#[tauri::command]
pub async fn generate_agent_completion(
    pubkey: String,
    prompt: String,
    system_prompt: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let (resolved_acp, agent_command, discovery) = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let records = load_managed_agents(&app)?;
        let record = records
            .iter()
            .find(|r| r.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?;

        let resolved = resolve_command(&record.acp_command)
            .ok_or_else(|| missing_command_message(&record.acp_command, "ACP harness command"))?;

        let personas = load_personas(&app).unwrap_or_default();
        let global = load_global_agent_config(&app).unwrap_or_default();
        let discovery = agent_model_discovery_config(record, &personas, &global).map_err(|e| {
            format!(
                "cannot run completion for {pubkey}: {}",
                user_facing_harness_error(&e)
            )
        })?;

        let resolved_agent = resolve_command(&discovery.command)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| discovery.command.clone());

        (resolved, resolved_agent, discovery)
    }; // store lock released — subprocess runs without holding the lock

    let AgentModelDiscoveryConfig {
        command: descriptor_command,
        args: agent_args,
        model,
        provider,
        env: merged_env,
        ..
    } = discovery;
    let mut merged_env = discovery_env_with_baked_floor(merged_env);

    // Mirror spawn_agent_child: the harness reads model/provider from its
    // runtime env vars, and those override the layered descriptor env.
    if let Some(meta) = known_acp_runtime(&descriptor_command) {
        for (key, value) in runtime_metadata_env_vars(
            meta.model_env_var,
            meta.provider_env_var,
            meta.provider_locked,
            model.as_deref(),
            provider.as_deref(),
        ) {
            merged_env.insert(key.to_string(), value.to_string());
        }
    }

    let mut helper_args = vec![
        "complete".to_string(),
        "--json".to_string(),
        "--prompt".to_string(),
        prompt,
    ];
    if let Some(system) = system_prompt.filter(|s| !s.trim().is_empty()) {
        helper_args.push("--system-prompt".to_string());
        helper_args.push(system);
    }
    if let Some(model) = model {
        helper_args.push("--model".to_string());
        helper_args.push(model);
    }

    let raw = run_acp_helper_subprocess(
        resolved_acp,
        agent_command,
        agent_args,
        merged_env,
        helper_args,
    )
    .await?;

    raw.get("text")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .ok_or_else(|| "buzz-acp complete returned no text".to_string())
}
