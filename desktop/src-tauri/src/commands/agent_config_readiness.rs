//! Managed-agent readiness preview contract.
//!
//! A preview is either a complete proposed standalone configuration or a patch
//! to one exact saved agent. Both are projected through the same command/env
//! owners used by create, update, and spawn before the existing readiness
//! predicate runs. Nothing in this module persists the projected record.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::managed_agents::{
    agent_readiness, resolve_effective_harness_descriptor, AgentDefinition, AgentReadiness,
    GlobalAgentConfig, ManagedAgentRecord, Requirement,
};

/// Complete configuration for a new standalone agent.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewAgentReadinessDraft {
    /// Harness command selected by the client. This is the same authoritative
    /// create field consumed by `create_managed_agent`; Custom commands use the
    /// same field rather than a parallel runtime identity.
    #[serde(default)]
    pub agent_command: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub env_vars: BTreeMap<String, String>,
}

/// Configuration patch for one exact saved agent.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExistingAgentReadinessDraft {
    pub pubkey: String,
    /// The same command/sentinel and explicit-override intent accepted by the
    /// production update path. Absent means keep the saved harness choice.
    #[serde(default)]
    pub agent_command: Option<String>,
    #[serde(default)]
    pub harness_override: bool,
    #[serde(default, deserialize_with = "crate::util::double_option")]
    pub model: Option<Option<String>>,
    #[serde(default, deserialize_with = "crate::util::double_option")]
    pub provider: Option<Option<String>>,
    /// Absent means keep the saved map; present means replace it, matching
    /// `UpdateManagedAgentRequest`.
    #[serde(default)]
    pub env_vars: Option<BTreeMap<String, String>>,
}

/// An unsaved readiness proposal, discriminated so a missing edit target can
/// never silently become a create preview.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentReadinessDraft {
    New { config: NewAgentReadinessDraft },
    Existing { config: ExistingAgentReadinessDraft },
}

/// Presentation-ready readiness result.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentReadinessEvaluation {
    pub ready: bool,
    /// Existing surface-discriminated requirements tell the client which
    /// affordance can resolve each gap.
    pub requirements: Vec<Requirement>,
}

fn trim_to_option(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn readiness_to_evaluation(readiness: AgentReadiness) -> AgentReadinessEvaluation {
    match readiness {
        AgentReadiness::Ready => AgentReadinessEvaluation {
            ready: true,
            requirements: Vec::new(),
        },
        AgentReadiness::NotReady { requirements } => AgentReadinessEvaluation {
            ready: false,
            requirements,
        },
    }
}

fn apply_existing_draft(
    record: &mut ManagedAgentRecord,
    draft: ExistingAgentReadinessDraft,
    definitions: &[AgentDefinition],
) -> Result<(), String> {
    crate::commands::managed_agent_definition::apply_model_provider_prompt_update(
        record,
        draft.model,
        draft.provider,
        None,
    )?;

    let inherit_transition = match draft.agent_command {
        Some(command) => crate::managed_agents::apply_agent_command_update(
            record,
            definitions,
            &command,
            draft.harness_override,
        ),
        None => false,
    };

    if let Some(ref env_vars) = draft.env_vars {
        crate::managed_agents::validate_user_env_keys(env_vars)?;
    }
    crate::managed_agents::apply_env_vars_then_effort_transition(
        record,
        draft.env_vars,
        inherit_transition,
    );
    Ok(())
}

fn build_new_record(
    draft: NewAgentReadinessDraft,
    definitions: &[AgentDefinition],
) -> Result<ManagedAgentRecord, String> {
    crate::managed_agents::validate_user_env_keys(&draft.env_vars)?;
    let picked_command = trim_to_option(draft.agent_command);
    let agent_command_override = crate::managed_agents::create_time_agent_command_override(
        None,
        definitions,
        picked_command.as_deref(),
        true,
    );
    let agent_command = crate::managed_agents::effective_agent_command(
        None,
        definitions,
        agent_command_override.as_deref(),
    );

    let inference = crate::commands::agents::create_fields::resolve_created_inference_config(
        draft.provider.as_deref(),
        draft.model.as_deref(),
        None,
    );
    let record: ManagedAgentRecord = serde_json::from_value(serde_json::json!({
        "pubkey": "",
        "name": "Readiness preview",
        "private_key_nsec": "",
        "relay_url": "",
        "acp_command": crate::managed_agents::DEFAULT_ACP_COMMAND,
        "agent_command": agent_command,
        "agent_command_override": agent_command_override,
        "agent_args": [],
        "mcp_command": "",
        "turn_timeout_seconds": crate::managed_agents::DEFAULT_AGENT_TURN_TIMEOUT_SECONDS,
        "parallelism": crate::managed_agents::DEFAULT_AGENT_PARALLELISM,
        "system_prompt": null,
        "model": inference.model,
        "provider": inference.provider,
        "env_vars": draft.env_vars,
        "relay_mesh": inference.relay_mesh,
        "created_at": "",
        "updated_at": "",
        "last_started_at": null,
        "last_stopped_at": null,
        "last_exit_code": null,
        "last_error": null
    }))
    .map_err(|error| format!("failed to build readiness preview: {error}"))?;
    Ok(record)
}

fn evaluate_record(
    record: &ManagedAgentRecord,
    definitions: &[AgentDefinition],
    global: &GlobalAgentConfig,
) -> Result<AgentReadinessEvaluation, String> {
    crate::managed_agents::effective_config::resolve_effective_config(record, definitions, global)
        .require_resolved()?;
    let descriptor = resolve_effective_harness_descriptor(record, definitions, global)
        .map_err(|error| crate::managed_agents::user_facing_harness_error(&error))?;
    let effective = crate::managed_agents::readiness::EffectiveAgentEnv {
        env: descriptor.env,
        config_file_path: crate::managed_agents::known_acp_runtime(&descriptor.command)
            .and_then(|runtime| runtime.config_file_path),
        effective_command: descriptor.command,
    };
    Ok(readiness_to_evaluation(agent_readiness(&effective)))
}

pub(super) fn evaluate_draft(
    draft: AgentReadinessDraft,
    saved_records: &[ManagedAgentRecord],
    definitions: &[AgentDefinition],
    global: &GlobalAgentConfig,
) -> Result<AgentReadinessEvaluation, String> {
    let record = match draft {
        AgentReadinessDraft::New { config } => build_new_record(config, definitions)?,
        AgentReadinessDraft::Existing { config } => {
            let mut record = saved_records
                .iter()
                .find(|record| record.pubkey == config.pubkey)
                .cloned()
                .ok_or_else(|| format!("agent {} not found", config.pubkey))?;
            apply_existing_draft(&mut record, config, definitions)?;
            record
        }
    };
    evaluate_record(&record, definitions, global)
}

#[cfg(test)]
#[path = "agent_config_readiness_tests.rs"]
mod tests;
