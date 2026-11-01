//! Harness LLM-provider inventory discovery.
//!
//! `discover_agent_providers` asks the selected ACP harness for its own list of
//! LLM providers (goose publishes one over the `_goose/unstable/providers/list`
//! custom request). The agent dialogs use it so the LLM-provider control offers
//! the providers the harness can actually run against, instead of a built-in
//! shortlist plus a free-text id.
//!
//! Kept separate from model discovery: the inventory is independent of the
//! selected provider, and model discovery short-circuits to provider HTTP APIs
//! (Anthropic, OpenRouter, …) which never see the harness at all.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::agent_model_process::run_agent_helper_command;
use crate::managed_agents::{
    discovery_env_with_baked_floor, is_reserved_env_key, merged_user_env, missing_command_message,
    normalize_agent_args, resolve_command, validate_user_env_keys, AgentProviderInfo,
    AgentProvidersResponse, DEFAULT_ACP_COMMAND,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverAgentProvidersInput {
    #[serde(default)]
    pub acp_command: Option<String>,
    pub agent_command: String,
    #[serde(default)]
    pub agent_args: Vec<String>,
    #[serde(default)]
    pub env_vars: BTreeMap<String, String>,
    /// Definition-level env from the harness definition (custom/preset).
    /// Merged below user `env_vars` so user overrides always win.
    #[serde(default)]
    pub definition_env: BTreeMap<String, String>,
}

/// Query the harness's LLM provider inventory for an unsaved agent form.
///
/// Mirrors `discover_agent_models`' env layering so a harness that needs
/// credentials to enumerate its providers sees exactly what a launch would.
/// Returns an empty list when the harness does not publish an inventory.
#[tauri::command]
pub async fn discover_agent_providers(
    input: DiscoverAgentProvidersInput,
) -> Result<AgentProvidersResponse, String> {
    validate_user_env_keys(&input.env_vars)?;
    // Definition env is caller-supplied at the same trust level as env_vars.
    validate_user_env_keys(&input.definition_env)?;

    let acp_command = input
        .acp_command
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_ACP_COMMAND);
    let resolved_acp = resolve_command(acp_command)
        .ok_or_else(|| missing_command_message(acp_command, "ACP harness command"))?;

    let agent_command = input.agent_command.trim();
    if agent_command.is_empty() {
        return Err("agent command is required for provider discovery".to_string());
    }
    let agent_args = normalize_agent_args(agent_command, input.agent_args);

    // Reserved keys are stripped from definition env, matching the same filter
    // applied at spawn and in model discovery.
    let mut definition_env = BTreeMap::new();
    for (key, value) in &input.definition_env {
        if !is_reserved_env_key(key) {
            definition_env.insert(key.clone(), value.clone());
        }
    }
    let merged_env = merged_user_env(&definition_env, &input.env_vars);
    let merged_env = discovery_env_with_baked_floor(merged_env);

    let raw = run_agent_helper_command(
        resolved_acp,
        "providers",
        agent_command.to_string(),
        agent_args,
        merged_env,
    )
    .await?;

    Ok(normalize_agent_providers(&raw))
}

/// Normalize `buzz-acp providers --json` output for the frontend.
///
/// Entries without a usable id are dropped rather than rendered as blank rows.
fn normalize_agent_providers(raw: &serde_json::Value) -> AgentProvidersResponse {
    let providers = raw["providers"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let id = entry.get("id").and_then(|v| v.as_str())?.trim();
                    if id.is_empty() {
                        return None;
                    }
                    Some(AgentProviderInfo {
                        id: id.to_string(),
                        name: entry
                            .get("name")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        configured: entry
                            .get("configured")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false),
                        default_model: entry
                            .get("defaultModel")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        acp: entry.get("acp").and_then(|v| v.as_bool()).unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    AgentProvidersResponse {
        agent_name: raw["agent"]["name"]
            .as_str()
            .unwrap_or("unknown")
            .to_string(),
        agent_version: raw["agent"]["version"]
            .as_str()
            .unwrap_or("unknown")
            .to_string(),
        providers,
    }
}

#[cfg(test)]
#[path = "agent_llm_providers_tests.rs"]
mod tests;
