//! Relay-mesh "Run on relay mesh" agent preset. Kept in a sibling file so
//! `mod.rs` stays under the 500-line budget; `#[path]`-included from there.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{
    relay_mesh_api_base_url, MESH_AGENT_MCP_COMMAND, MESH_AGENT_PROVIDER_ID,
    RELAY_MESH_API_KEY_PLACEHOLDER,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeshAgentPresetRequest {
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeshAgentPreset {
    pub provider_id: String,
    pub label: String,
    pub acp_command: String,
    pub agent_command: String,
    pub agent_args: Vec<String>,
    pub mcp_command: String,
    pub model: String,
    pub env_vars: BTreeMap<String, String>,
}

pub fn agent_preset(request: MeshAgentPresetRequest) -> Result<MeshAgentPreset, String> {
    let model = request.model_id.trim();
    if model.is_empty() {
        return Err("modelId is required".to_string());
    }
    // Run on sprout-agent, not the global default (goose). Source command +
    // MCP from the catalog so this can't drift from the provider definition.
    let sprout_agent = crate::managed_agents::known_acp_runtime_exact(MESH_AGENT_PROVIDER_ID);
    let agent_command = sprout_agent
        .and_then(|p| p.commands.first().copied())
        .unwrap_or(MESH_AGENT_PROVIDER_ID)
        .to_string();
    let mcp_command = sprout_agent
        .and_then(|p| p.mcp_command)
        .unwrap_or(MESH_AGENT_MCP_COMMAND)
        .to_string();
    Ok(MeshAgentPreset {
        provider_id: "relay-mesh".to_string(),
        label: "Relay mesh".to_string(),
        acp_command: crate::managed_agents::DEFAULT_ACP_COMMAND.to_string(),
        agent_command,
        agent_args: Vec::new(),
        mcp_command,
        model: model.to_string(),
        env_vars: BTreeMap::from([
            ("SPROUT_AGENT_PROVIDER".to_string(), "openai".to_string()),
            (
                "OPENAI_COMPAT_BASE_URL".to_string(),
                relay_mesh_api_base_url()?,
            ),
            ("OPENAI_COMPAT_MODEL".to_string(), model.to_string()),
            (
                "OPENAI_COMPAT_API_KEY".to_string(),
                RELAY_MESH_API_KEY_PLACEHOLDER.to_string(),
            ),
            ("OPENAI_COMPAT_API".to_string(), "chat".to_string()),
        ]),
    })
}
