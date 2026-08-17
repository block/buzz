//! Build kind 47200/47201 Agent Studio config events alongside persona publishes.

use buzz_core_pkg::kind::{KIND_AGENT_CONFIG_CREATED, KIND_AGENT_CONFIG_UPDATED};
use nostr::{EventBuilder, Kind, Tag};
use serde::Serialize;

use super::{persona_events::persona_d_tag, AgentDefinition};

/// JSON body for [`KIND_AGENT_CONFIG_CREATED`] / [`KIND_AGENT_CONFIG_UPDATED`].
#[derive(Debug, Clone, Serialize)]
struct AgentConfigPayload<'a> {
    agent_id: &'a str,
    config_json: String,
}

/// Build a replaceable Agent Studio config event for the given persona record.
pub fn build_agent_config_event(
    persona: &AgentDefinition,
    is_create: bool,
) -> Result<EventBuilder, String> {
    let agent_id = persona_d_tag(persona);
    let config_json = serde_json::to_string(persona)
        .map_err(|e| format!("failed to serialize persona for agent config: {e}"))?;
    let content = serde_json::to_string(&AgentConfigPayload {
        agent_id: &agent_id,
        config_json,
    })
    .map_err(|e| format!("failed to serialize agent config payload: {e}"))?;
    let kind = if is_create {
        KIND_AGENT_CONFIG_CREATED
    } else {
        KIND_AGENT_CONFIG_UPDATED
    };
    Ok(EventBuilder::new(Kind::Custom(kind as u16), content).tags(vec![Tag::identifier(agent_id)]))
}
