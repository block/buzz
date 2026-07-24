use std::collections::BTreeMap;

use serde::Deserialize;

use crate::managed_agents::{
    apply_runtime_security_env, known_acp_runtime, merged_user_env, runtime_metadata_env_vars,
    ManagedAgentRecord,
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
    agent_command: &str,
) -> SavedAgentModelDiscoveryConfig {
    let mut derived_env = BTreeMap::new();
    if let Some(meta) = known_acp_runtime(agent_command) {
        for (key, value) in runtime_metadata_env_vars(
            meta.model_env_var,
            meta.provider_env_var,
            meta.provider_locked,
            meta.locked_provider_id,
            record.model.as_deref(),
            record.provider.as_deref(),
        ) {
            derived_env.insert(key.to_string(), value.to_string());
        }
    }

    let mut env = merged_user_env(&derived_env, &record.env_vars);
    apply_runtime_security_env(&mut env, known_acp_runtime(agent_command));
    SavedAgentModelDiscoveryConfig {
        model: record.model.clone(),
        provider: record.provider.clone(),
        env,
    }
}
