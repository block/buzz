//! Model-discovery configuration resolution for the agent-models commands.
//!
//! Two entry points, one per call shape: [`agent_model_discovery_config`]
//! resolves a *saved* agent through the same descriptor/model resolvers spawn
//! uses, and [`draft_agent_model_discovery_env`] derives the env for an unsaved
//! form. Both are pure so the regression tests can bind the exact values the
//! commands consume.
//!
//! Included from `agent_models.rs` via `#[path]`, so `super::*` resolves
//! against that module (the `agent_models_tests.rs` convention).

use std::collections::BTreeMap;

use crate::managed_agents::known_acp_runtime;

/// Everything `get_agent_models` needs from the record + context, resolved in
/// one pure step so the linked-agent regression test can bind the exact values
/// the command consumes.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct AgentModelDiscoveryConfig {
    /// Effective harness command (descriptor-resolved), for `resolve_command`.
    pub(super) command: String,
    /// Effective harness args (descriptor-resolved).
    pub(super) args: Vec<String>,
    /// Model from the authoritative resolver spawn uses — linked instances
    /// read their definition, never stale `record.model` bytes.
    pub(super) model: Option<String>,
    /// Provider from the same authoritative resolver — never stale
    /// `record.provider` bytes for linked instances.
    pub(super) provider: Option<String>,
    /// The runtime's provider env var (e.g. `GOOSE_PROVIDER`), so discovery
    /// can recover the provider from the env when the resolver yields none.
    /// `None` for runtimes that do not take a provider, or an unknown command.
    pub(super) provider_env_var: Option<&'static str>,
    /// The descriptor's fully layered env (definition/persona/global/agent).
    pub(super) env: BTreeMap<String, String>,
}

/// Resolve the model-discovery config for a saved agent — the descriptor-backed
/// successor to the old `saved_agent_model_discovery_config`.
///
/// Command/args/env come from `resolve_effective_harness_descriptor` (the same
/// resolver as `spawn_agent_child`); model/provider come from
/// `resolve_effective_model_provider` (#1968's definition-authoritative
/// contract) — linked instances read their definition, never a stale
/// materialized `record.model`/`record.provider`, so discovery cannot query a
/// provider this agent will not actually launch with. Definition-less
/// instances keep their own record values, matching spawn's
/// `resolve_definition_less` arm. When the resolver yields no provider,
/// `effective_discovery_provider` recovers the provider the agent will
/// actually launch with from the runtime's own provider env var, read out of
/// the descriptor env (which already layers definition/persona/global values
/// the same way spawn does).
///
/// Returns `Err("DANGLING_HARNESS_ID:<id>")` from the descriptor resolver when
/// the harness id no longer exists; the caller routes it through
/// `model_discovery_error`.
pub(super) fn agent_model_discovery_config(
    record: &crate::managed_agents::ManagedAgentRecord,
    personas: &[crate::managed_agents::AgentDefinition],
    global: &crate::managed_agents::GlobalAgentConfig,
) -> Result<AgentModelDiscoveryConfig, String> {
    let descriptor =
        crate::managed_agents::resolve_effective_harness_descriptor(record, personas, global)?;
    let (model, provider) =
        crate::managed_agents::resolve_effective_model_provider(record, personas, global);
    let provider_env_var =
        known_acp_runtime(&descriptor.command).and_then(|meta| meta.provider_env_var);

    Ok(AgentModelDiscoveryConfig {
        command: descriptor.command,
        args: descriptor.args,
        model,
        provider,
        provider_env_var,
        env: descriptor.env,
    })
}

/// Derive the discovery env for an unsaved ("draft") agent configuration.
///
/// Mirrors the layering `agent_model_discovery_config` takes from the harness
/// descriptor, but sources the provider from form input: runtime-derived
/// provider env var → definition env → user env vars, so user overrides always
/// win. Extracted so the draft path has the same tested seam as the saved one.
pub(super) fn draft_agent_model_discovery_env(
    agent_command: &str,
    provider: Option<&str>,
    definition_env: &BTreeMap<String, String>,
    env_vars: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut derived_env = BTreeMap::new();
    if let Some(meta) = known_acp_runtime(agent_command) {
        let provider = provider.map(str::trim).filter(|value| !value.is_empty());
        if !meta.provider_locked {
            if let (Some(env_key), Some(provider)) = (meta.provider_env_var, provider) {
                derived_env.insert(env_key.to_string(), provider.to_string());
            }
        }
    }
    // Reserved keys are stripped from definition env, matching the same filter
    // applied at spawn.
    let mut filtered_definition_env = BTreeMap::new();
    for (key, value) in definition_env {
        if !crate::managed_agents::is_reserved_env_key(key) {
            filtered_definition_env.insert(key.clone(), value.clone());
        }
    }
    let merged_with_def =
        crate::managed_agents::merged_user_env(&derived_env, &filtered_definition_env);
    crate::managed_agents::merged_user_env(&merged_with_def, env_vars)
}
