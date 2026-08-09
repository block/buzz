use std::collections::BTreeMap;

use serde::Deserialize;

use crate::managed_agents::{
    known_acp_runtime, record_agent_command_with_preferred_runtime, resolve_effective_agent_env,
    resolve_effective_model_provider, AgentDefinition, GlobalAgentConfig, ManagedAgentRecord,
};

#[derive(Debug, PartialEq, Eq)]
pub(super) struct SavedAgentModelDiscoveryConfig {
    pub(super) model: Option<String>,
    pub(super) provider: Option<String>,
    pub(super) env: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Form state used to discover models before an agent record exists.
pub(crate) struct DiscoverAgentModelsInput {
    #[serde(default)]
    pub(super) acp_command: Option<String>,
    pub(super) agent_command: String,
    #[serde(default)]
    pub(super) agent_args: Vec<String>,
    #[serde(default)]
    pub(super) provider: Option<String>,
    #[serde(default)]
    pub(super) env_vars: BTreeMap<String, String>,
}

pub(super) fn saved_agent_model_discovery_config(
    record: &ManagedAgentRecord,
    personas: &[AgentDefinition],
    global: &GlobalAgentConfig,
) -> SavedAgentModelDiscoveryConfig {
    let agent_command = record_agent_command_with_preferred_runtime(
        record,
        personas,
        global.preferred_runtime.as_deref(),
    );
    let runtime = known_acp_runtime(&agent_command);
    let effective = resolve_effective_agent_env(record, personas, runtime, global);
    let (model, provider) = resolve_effective_model_provider(record, personas, global);
    SavedAgentModelDiscoveryConfig {
        model: model.map(str::to_string),
        provider: provider.map(str::to_string),
        env: effective.env,
    }
}
