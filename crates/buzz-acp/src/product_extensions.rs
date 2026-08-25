//! Product-specific ACP integrations composed behind generic turn contracts.

use std::sync::Arc;

use crate::turn_observer::TurnExtension;

#[path = "business_agent.rs"]
mod business_agent;
#[path = "business_response.rs"]
mod business_response;

pub(crate) fn load_from_env(agent_command: &str) -> Result<Option<Arc<dyn TurnExtension>>, String> {
    let extension = business_agent::BusinessAgentHostConfig::from_env()?;
    if extension.is_some()
        && crate::config::normalize_agent_command_identity(agent_command) != "buzz-agent"
    {
        tracing::warn!(
            runtime = agent_command,
            "Turn extension is using a general-purpose ACP runtime; ordinary MCP servers are removed, but the runtime may expose its own built-in tools"
        );
    }
    Ok(extension.map(|extension| Arc::new(extension) as Arc<dyn TurnExtension>))
}
