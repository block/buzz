use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

/// Maximum number of enabled user-configured MCP servers. The bundled
/// `buzz-dev-mcp` occupies the sixteenth `buzz-agent` MCP slot.
pub(crate) const MAX_USER_MCP_SERVERS: usize = 15;

/// Maximum byte length of an MCP server name. Mirrors `MAX_NAME_LEN` in
/// `buzz-agent/src/mcp.rs`.
const MAX_SERVER_NAME_LEN: usize = 128;

/// The built-in MCP server name reserved by the buzz-dev-mcp sidecar.
/// User-configured servers must not use this name — buzz-acp always appends
/// the built-in after user servers, making it a duplicate at spawn time.
const RESERVED_SERVER_NAME: &str = "buzz-dev-mcp";

/// An environment variable passed to an MCP subprocess.
///
/// This deliberately mirrors the ACP `EnvVar` wire shape so the effective
/// configuration can be serialized without translating local secrets through
/// relay event content.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct McpServerEnvVar {
    pub name: String,
    pub value: String,
}

/// A locally persisted stdio MCP server configuration.
///
/// Entries are layered `global < definition < agent` by name. A higher layer
/// replaces an entire entry; setting `enabled` to `false` masks a lower entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct McpServerConfig {
    pub name: String,
    /// Required for enabled servers. Disabled entries may omit it because
    /// their only purpose is to mask a lower-precedence server by name.
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Vec<McpServerEnvVar>,
    #[serde(default = "default_mcp_server_enabled")]
    pub enabled: bool,
}

/// The resolved MCP shape sent to `buzz-acp` and provider deployments.
/// `enabled` is local layering metadata, so it deliberately does not cross
/// the process or provider boundary.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct McpServerTransport {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<McpServerEnvVar>,
}

impl From<McpServerConfig> for McpServerTransport {
    fn from(server: McpServerConfig) -> Self {
        Self {
            name: server.name,
            command: server.command,
            args: server.args,
            env: server.env,
        }
    }
}

pub(crate) fn mcp_server_transport(servers: Vec<McpServerConfig>) -> Vec<McpServerTransport> {
    servers.into_iter().map(Into::into).collect()
}

fn default_mcp_server_enabled() -> bool {
    true
}

/// Validate one persisted MCP layer at every IPC save boundary.
///
/// Checks:
/// - Names match the buzz-agent grammar: 1–128 bytes, ASCII alphanumeric,
///   `_`, or `-` only, no `__` substring. This mirrors `valid_name` in
///   `buzz-agent/src/mcp.rs` so save-time rejection matches spawn-time
///   rejection.
/// - Names must not be `buzz-dev-mcp` — that slot is reserved for the
///   bundled sidecar and would become a duplicate when buzz-acp appends it.
/// - No duplicate names within the layer.
/// - Enabled servers must have a non-empty command.
/// - No NUL bytes; individual values under the length ceiling.
/// - Total payload under the overall byte budget.
/// - At most `MAX_USER_MCP_SERVERS` enabled entries (layer-level cap;
///   the merged effective-server cap is enforced separately in
///   `merge_mcp_servers`).
pub(crate) fn validate_mcp_servers(servers: &[McpServerConfig]) -> Result<(), String> {
    let mut names = HashSet::new();
    let mut enabled_count = 0usize;
    let mut total_bytes = 0usize;
    for server in servers {
        // Grammar validation — must match buzz-agent's `valid_name` exactly.
        if server.name.is_empty() {
            return Err("MCP server name is required".to_string());
        }
        if server.name.len() > MAX_SERVER_NAME_LEN {
            return Err(format!(
                "MCP server name `{}` exceeds the maximum length ({MAX_SERVER_NAME_LEN} bytes)",
                server.name
            ));
        }
        if !server
            .name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(format!(
                "MCP server name `{}` contains invalid characters (only ASCII alphanumeric, `_`, and `-` are allowed)",
                server.name
            ));
        }
        if server.name.contains("__") {
            return Err(format!(
                "MCP server name `{}` cannot contain `__`",
                server.name
            ));
        }
        if server.name == RESERVED_SERVER_NAME {
            return Err(format!(
                "MCP server name `{RESERVED_SERVER_NAME}` is reserved"
            ));
        }

        if !names.insert(server.name.as_str()) {
            return Err(format!("MCP server names must be unique: {}", server.name));
        }
        if server.enabled && server.command.trim().is_empty() {
            return Err(format!("MCP server `{}` command is required", server.name));
        }
        if server.enabled {
            enabled_count += 1;
        }

        for (field, value) in std::iter::once(("name", &server.name))
            .chain((!server.command.is_empty()).then_some(("command", &server.command)))
            .chain(server.args.iter().map(|arg| ("argument", arg)))
        {
            if value.contains('\0') {
                return Err(format!(
                    "MCP server `{}` {field} cannot contain NUL bytes",
                    server.name
                ));
            }
            if value.len() > crate::managed_agents::env_vars::MAX_ENV_VALUE_BYTES {
                return Err(format!(
                    "MCP server `{}` {field} exceeds the maximum allowed length ({} bytes)",
                    server.name,
                    crate::managed_agents::env_vars::MAX_ENV_VALUE_BYTES
                ));
            }
            total_bytes = total_bytes.saturating_add(value.len());
        }

        let mut env = BTreeMap::new();
        for variable in &server.env {
            if env
                .insert(variable.name.clone(), variable.value.clone())
                .is_some()
            {
                return Err(format!(
                    "MCP server `{}` has duplicate env var `{}`",
                    server.name, variable.name
                ));
            }
        }
        crate::managed_agents::validate_user_env_keys(&env)?;
        total_bytes = total_bytes.saturating_add(
            env.iter()
                .map(|(key, value)| key.len() + value.len())
                .sum::<usize>(),
        );
    }

    if enabled_count > MAX_USER_MCP_SERVERS {
        return Err(format!(
            "MCP server layer has {enabled_count} enabled servers; limit is {MAX_USER_MCP_SERVERS}"
        ));
    }
    if total_bytes > crate::managed_agents::env_vars::MAX_ENV_TOTAL_BYTES {
        return Err(format!(
            "total MCP server payload is {total_bytes} bytes; limit is {}",
            crate::managed_agents::env_vars::MAX_ENV_TOTAL_BYTES
        ));
    }
    Ok(())
}

/// Validate and replace an optional MCP layer from an update request.
pub(crate) fn replace_mcp_servers(
    current: &mut Vec<McpServerConfig>,
    replacement: &Option<Vec<McpServerConfig>>,
) -> Result<(), String> {
    if let Some(replacement) = replacement {
        validate_mcp_servers(replacement)?;
        *current = replacement.clone();
    }
    Ok(())
}

/// Resolve global, definition, and per-agent MCP layers. Higher layers replace
/// whole entries by name, including disabled entries, so a disabled override
/// masks an inherited server instead of allowing it to leak through.
pub(crate) fn merge_mcp_servers(
    global: &[McpServerConfig],
    definition: &[McpServerConfig],
    agent: &[McpServerConfig],
) -> Result<Vec<McpServerConfig>, String> {
    let mut merged = BTreeMap::new();
    for layer in [global, definition, agent] {
        for server in layer {
            merged.insert(server.name.clone(), server.clone());
        }
    }
    let effective: Vec<McpServerConfig> = merged
        .into_values()
        .filter(|server| server.enabled)
        .collect();
    if effective.len() > MAX_USER_MCP_SERVERS {
        return Err(format!(
            "effective MCP server count is {} but the maximum is {MAX_USER_MCP_SERVERS}",
            effective.len()
        ));
    }
    Ok(effective)
}

/// Resolve MCP configuration only for the bundled `buzz-agent` runtime.
/// Other ACP runtimes keep their MCP configuration in their own config files.
pub(crate) fn effective_buzz_agent_mcp_servers(
    record: &super::ManagedAgentRecord,
    personas: &[super::AgentDefinition],
    global: &[McpServerConfig],
    effective_command: &str,
) -> Result<Vec<McpServerConfig>, String> {
    if super::super::known_acp_runtime(effective_command).map(|runtime| runtime.id)
        != Some("buzz-agent")
    {
        return Ok(Vec::new());
    }
    let definition = record
        .persona_id
        .as_deref()
        .and_then(|id| personas.iter().find(|persona| persona.id == id))
        .map(|persona| persona.mcp_servers.as_slice())
        .unwrap_or_default();
    merge_mcp_servers(global, definition, &record.mcp_servers)
}

/// Validate that the prospective effective-merge count stays within the
/// [`MAX_USER_MCP_SERVERS`] cap. Called at agent create/update to prevent a
/// record that passes per-layer validation but exceeds the cap after the
/// global < definition < agent merge. Non–buzz-agent runtimes skip the check
/// (they don't use `BUZZ_ACP_MCP_SERVERS`).
pub(crate) fn validate_effective_mcp_cap(
    record: &super::ManagedAgentRecord,
    personas: &[super::AgentDefinition],
    global: &[McpServerConfig],
    effective_command: &str,
) -> Result<(), String> {
    // effective_buzz_agent_mcp_servers returns Ok(empty) for non-buzz-agent
    // runtimes, so the cap check only fires for buzz-agent.
    effective_buzz_agent_mcp_servers(record, personas, global, effective_command)?;
    Ok(())
}

/// Validate the effective MCP cap across all existing agent records against a
/// prospective inherited layer (global or persona). Called at global-config
/// save and persona update to prevent an inherited-layer mutation that would
/// silently push an existing agent over the cap.
///
/// Non–buzz-agent runtimes are skipped (they don't use `BUZZ_ACP_MCP_SERVERS`).
/// Returns the first offending agent's name in the error message.
pub(crate) fn validate_effective_mcp_cap_for_records(
    records: &[super::ManagedAgentRecord],
    personas: &[super::AgentDefinition],
    global: &[McpServerConfig],
) -> Result<(), String> {
    for record in records {
        let effective_cmd = crate::managed_agents::record_agent_command(record, personas);
        if let Err(merge_err) =
            effective_buzz_agent_mcp_servers(record, personas, global, &effective_cmd)
        {
            return Err(format!(
                "saving would push agent `{}` over the MCP server limit: {merge_err}",
                record.name
            ));
        }
    }
    Ok(())
}
