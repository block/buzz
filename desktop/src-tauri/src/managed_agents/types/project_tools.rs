use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::managed_agents::project_connections::ProjectConnectionScope;

const MAX_TOOL_REQUIREMENTS: usize = 32;
const MAX_REQUIREMENT_ID_BYTES: usize = 64;
const MAX_REQUIREMENT_LABEL_BYTES: usize = 128;
const MAX_CAPABILITY_BYTES: usize = 128;

/// A portable tool capability declared by an agent definition.
///
/// Definitions describe what the agent needs. Project-owned connection
/// records provide the executable, endpoint details, and local credentials.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct AgentToolRequirement {
    /// Stable definition-local identifier used by instance bindings.
    pub id: String,
    /// User-facing requirement name.
    pub label: String,
    /// MCP capability identifier, in the form `mcp.tool.<stable-id>`.
    pub capability: String,
    /// Whether an instance may start without a matching connection.
    #[serde(default = "default_tool_requirement_required")]
    pub required: bool,
}

fn default_tool_requirement_required() -> bool {
    true
}

/// Project assignment for one managed-agent instance.
///
/// Connection ownership stops at `project_address`. `channel_id` scopes the
/// agent's work, but does not create a second set of Project credentials when
/// the discussion channel changes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct AgentProjectScope {
    pub relay_url: String,
    pub operator_pubkey: String,
    /// Durable NIP-MP Project coordinate. Legacy one-repository Projects use
    /// their NIP-34 repository coordinate.
    #[serde(alias = "repoAddress")]
    pub project_address: String,
    pub channel_id: String,
}

impl From<&AgentProjectScope> for ProjectConnectionScope {
    fn from(scope: &AgentProjectScope) -> Self {
        Self {
            relay_url: scope.relay_url.clone(),
            operator_pubkey: scope.operator_pubkey.clone(),
            project_address: scope.project_address.clone(),
        }
    }
}

fn valid_stable_id(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_bytes
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn is_unsafe_object_key(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "__proto__" | "constructor" | "prototype"
    )
}

/// Validate a complete portable requirement set.
pub fn validate_tool_requirements(requirements: &[AgentToolRequirement]) -> Result<(), String> {
    if requirements.len() > MAX_TOOL_REQUIREMENTS {
        return Err(format!(
            "An agent definition may declare at most {MAX_TOOL_REQUIREMENTS} tool requirements."
        ));
    }

    let mut ids = BTreeSet::new();
    for requirement in requirements {
        if !valid_stable_id(&requirement.id, MAX_REQUIREMENT_ID_BYTES)
            || requirement.id != requirement.id.to_ascii_lowercase()
            || is_unsafe_object_key(&requirement.id)
        {
            return Err(format!(
                "{:?} is not a valid tool requirement id.",
                requirement.id
            ));
        }
        if !ids.insert(requirement.id.to_ascii_lowercase()) {
            return Err(format!(
                "Tool requirement ids must be unique, ignoring case: {:?}.",
                requirement.id
            ));
        }
        if requirement.label.trim().is_empty()
            || requirement.label.len() > MAX_REQUIREMENT_LABEL_BYTES
            || requirement.label.chars().any(char::is_control)
        {
            return Err(format!(
                "Tool requirement {:?} has an invalid label.",
                requirement.id
            ));
        }
        let Some(capability_id) = requirement.capability.strip_prefix("mcp.tool.") else {
            return Err(format!(
                "Tool requirement {:?} must use an MCP tool capability.",
                requirement.id
            ));
        };
        if requirement.capability.len() > MAX_CAPABILITY_BYTES
            || !valid_stable_id(capability_id, MAX_CAPABILITY_BYTES - "mcp.tool.".len())
            || is_unsafe_object_key(capability_id)
        {
            return Err(format!(
                "Tool requirement {:?} has an invalid capability.",
                requirement.id
            ));
        }
    }
    Ok(())
}

/// Validate fields specific to an agent's Project assignment.
pub fn validate_agent_project_scope(scope: &AgentProjectScope) -> Result<(), String> {
    uuid::Uuid::parse_str(&scope.channel_id)
        .map_err(|_| "Choose a valid Project discussion channel.".to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn requirement(id: &str, capability: &str) -> AgentToolRequirement {
        AgentToolRequirement {
            id: id.to_string(),
            label: "Analytics reports".to_string(),
            capability: capability.to_string(),
            required: true,
        }
    }

    #[test]
    fn accepts_distinct_portable_mcp_requirements() {
        assert!(validate_tool_requirements(&[
            requirement("analytics", "mcp.tool.analytics.weekly_summary"),
            requirement("crm", "mcp.tool.crm.accounts.read"),
        ])
        .is_ok());
    }

    #[test]
    fn rejects_count_case_collisions_and_unsafe_object_keys() {
        let too_many = vec![requirement("analytics", "mcp.tool.analytics"); 33];
        assert!(validate_tool_requirements(&too_many).is_err());
        assert!(validate_tool_requirements(&[
            requirement("Analytics", "mcp.tool.analytics"),
            requirement("analytics", "mcp.tool.analytics"),
        ])
        .is_err());
        for id in ["__proto__", "Constructor", "prototype"] {
            assert!(validate_tool_requirements(&[requirement(id, "mcp.tool.safe")]).is_err());
        }
    }

    #[test]
    fn rejects_invalid_labels_and_capabilities() {
        let mut invalid_label = requirement("analytics", "mcp.tool.analytics");
        invalid_label.label = "\n".to_string();
        assert!(validate_tool_requirements(&[invalid_label]).is_err());
        assert!(validate_tool_requirements(&[requirement("analytics", "analytics")]).is_err());
        assert!(
            validate_tool_requirements(&[requirement("analytics", "mcp.tool.__proto__")]).is_err()
        );
    }

    #[test]
    fn validates_project_channel_without_changing_connection_ownership() {
        let scope = AgentProjectScope {
            relay_url: "ws://127.0.0.1:3000".to_string(),
            operator_pubkey: "a".repeat(64),
            project_address: format!("30621:{}:analytics", "a".repeat(64)),
            channel_id: uuid::Uuid::nil().to_string(),
        };
        assert!(validate_agent_project_scope(&scope).is_ok());
        let mut other_channel = scope.clone();
        other_channel.channel_id = uuid::Uuid::new_v4().to_string();
        assert_eq!(scope.project_address, other_channel.project_address);
        assert!(validate_agent_project_scope(&other_channel).is_ok());
    }
}
