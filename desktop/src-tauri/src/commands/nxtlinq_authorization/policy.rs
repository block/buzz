use std::collections::HashSet;

use serde_json::{Map, Value};

use super::{NxtlinqManifestPolicyDraft, NXTLINQ_AUDIENCE, NXTLINQ_STRUCTURED_SCOPE};

const BUZZ_DEV_MCP_SERVER: &str = "buzz-dev-mcp";
const BUNDLED_BUZZ_MCP_TOOLS: &[&str] = &[
    "read_file",
    "str_replace",
    "shell",
    "buzz_message_send",
    "nxtlinq_setup",
    "todo",
    "_Stop",
    "_PostCompact",
];
pub(super) const REQUIRED_SENSITIVE_EXCLUDES: &[&str] = &[
    ".env*",
    "**/.env*",
    ".npmrc",
    "**/.npmrc",
    ".netrc",
    "**/.netrc",
    ".pypirc",
    "**/.pypirc",
    ".git-credentials",
    "**/.git-credentials",
    ".git/**",
    "nxtlinq/**",
    ".aws/**",
    "**/.aws/**",
    ".docker/**",
    "**/.docker/**",
    "credentials",
    "**/credentials",
    "**/credentials/**",
    "**/.ssh/**",
    "*.pem",
    "**/*.pem",
    "*.key",
    "**/*.key",
    "*.p12",
    "**/*.p12",
];
const FORBIDDEN_TERMINAL_ENVIRONMENT_NAMES: &[&str] =
    &["BUZZ_PRIVATE_KEY", "NOSTR_PRIVATE_KEY", "BUZZ_AUTH_TAG"];

fn nonempty(value: &str, label: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{label} must not be empty"))
    } else {
        Ok(())
    }
}

fn validate_allowed_fields(
    capability: &Map<String, Value>,
    index: usize,
    allowed: &[&str],
) -> Result<(), String> {
    for name in capability.keys() {
        if !allowed.contains(&name.as_str()) {
            return Err(format!(
                "capabilities[{index}] contains unsupported constraint {name}"
            ));
        }
    }
    Ok(())
}

fn string_array<'a>(
    capability: &'a Map<String, Value>,
    index: usize,
    name: &str,
    required: bool,
) -> Result<Vec<&'a str>, String> {
    let Some(value) = capability.get(name) else {
        if required {
            return Err(format!(
                "capabilities[{index}].{name} must be a non-empty string array"
            ));
        }
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| format!("capabilities[{index}].{name} must be a string array"))?;
    if required && values.is_empty() {
        return Err(format!(
            "capabilities[{index}].{name} must be a non-empty string array"
        ));
    }
    values
        .iter()
        .enumerate()
        .map(|(value_index, value)| {
            value
                .as_str()
                .filter(|text| !text.trim().is_empty())
                .ok_or_else(|| {
                    format!(
                        "capabilities[{index}].{name}[{value_index}] must be a non-empty string"
                    )
                })
        })
        .collect()
}

fn validate_approval_required(capability: &Map<String, Value>, index: usize) -> Result<(), String> {
    let approval_required = capability.get("approvalRequired");
    if approval_required.is_some_and(|value| !value.is_boolean()) {
        return Err(format!(
            "capabilities[{index}].approvalRequired must be a boolean"
        ));
    }
    if approval_required.and_then(Value::as_bool) == Some(true) {
        return Err(format!(
            "capabilities[{index}].approvalRequired=true is unsupported by conversational setup"
        ));
    }
    Ok(())
}

fn is_windows_drive_absolute(pattern: &str) -> bool {
    let bytes = pattern.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}

fn validate_project_relative_pattern(
    pattern: &str,
    index: usize,
    field: &str,
) -> Result<(), String> {
    if pattern.contains('\0') {
        return Err(format!("capabilities[{index}].{field} contains a NUL byte"));
    }
    if pattern.starts_with('/') || pattern.starts_with('\\') || is_windows_drive_absolute(pattern) {
        return Err(format!(
            "capabilities[{index}].{field} must use project-relative patterns, not {pattern}"
        ));
    }
    if pattern
        .split(['/', '\\'])
        .any(|component| component == "..")
    {
        return Err(format!(
            "capabilities[{index}].{field} must not contain parent-directory segments"
        ));
    }
    Ok(())
}

fn validate_filesystem_capability(
    capability: &Map<String, Value>,
    index: usize,
    writable: bool,
) -> Result<(), String> {
    let allowed: &[&str] = if writable {
        &["type", "include", "exclude", "approvalRequired"]
    } else {
        &["type", "include", "exclude"]
    };
    validate_allowed_fields(capability, index, allowed)?;
    let includes = string_array(capability, index, "include", true)?;
    let excludes = string_array(capability, index, "exclude", false)?;
    for pattern in includes {
        validate_project_relative_pattern(pattern, index, "include")?;
    }
    for pattern in excludes {
        validate_project_relative_pattern(pattern, index, "exclude")?;
    }
    let excludes = string_array(capability, index, "exclude", false)?;
    if REQUIRED_SENSITIVE_EXCLUDES
        .iter()
        .any(|required| !excludes.contains(required))
    {
        return Err(format!(
            "capabilities[{index}].exclude must contain the required sensitive-file exclusions"
        ));
    }
    if writable {
        validate_approval_required(capability, index)?;
    }
    Ok(())
}

fn valid_environment_name(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn validate_terminal_capability(
    capability: &Map<String, Value>,
    index: usize,
) -> Result<(), String> {
    validate_allowed_fields(
        capability,
        index,
        &["type", "commands", "environment", "approvalRequired"],
    )?;
    for command in string_array(capability, index, "commands", true)? {
        if command.contains('\0') {
            return Err(format!(
                "capabilities[{index}].commands must not contain NUL bytes"
            ));
        }
    }
    let environment = string_array(capability, index, "environment", false)?;
    for environment in &environment {
        if !valid_environment_name(environment) {
            return Err(format!(
                "capabilities[{index}].environment must contain environment variable names only"
            ));
        }
        if FORBIDDEN_TERMINAL_ENVIRONMENT_NAMES.contains(environment) {
            return Err(format!(
                "capabilities[{index}].environment cannot expose host identity variable {environment}"
            ));
        }
    }
    if !environment.contains(&"PATH") {
        return Err(format!(
            "capabilities[{index}].environment must include PATH for protected shell execution"
        ));
    }
    validate_approval_required(capability, index)
}

fn validate_mcp_capability(
    capability: &Map<String, Value>,
    index: usize,
    invoke: bool,
) -> Result<(), String> {
    let allowed: &[&str] = if invoke {
        &["type", "servers", "tools", "approvalRequired"]
    } else {
        &["type", "servers", "approvalRequired"]
    };
    validate_allowed_fields(capability, index, allowed)?;
    let servers = string_array(capability, index, "servers", true)?;
    if invoke {
        if servers.len() != 1 {
            return Err(format!(
                "capabilities[{index}].servers must contain exactly one server for mcp:invoke"
            ));
        }
        let tools = string_array(capability, index, "tools", true)?;
        if servers.contains(&BUZZ_DEV_MCP_SERVER)
            && tools
                .iter()
                .any(|tool| BUNDLED_BUZZ_MCP_TOOLS.contains(tool))
        {
            return Err(format!(
                "capabilities[{index}] must authorize bundled Buzz tools through filesystem or terminal capabilities, not mcp:invoke"
            ));
        }
    }
    validate_approval_required(capability, index)
}

pub(super) fn validate_policy(policy: &NxtlinqManifestPolicyDraft) -> Result<(), String> {
    nonempty(&policy.name, "manifest name")?;
    nonempty(&policy.version, "manifest version")?;
    if policy.scope.len() != 1 || policy.scope[0] != NXTLINQ_STRUCTURED_SCOPE {
        return Err(format!(
            "manifest scope must be exactly [{NXTLINQ_STRUCTURED_SCOPE}]"
        ));
    }
    if policy.aud.len() != 1 || policy.aud[0] != NXTLINQ_AUDIENCE {
        return Err(format!(
            "manifest audience must be exactly [{NXTLINQ_AUDIENCE}]"
        ));
    }
    if policy.capabilities.is_empty() {
        return Err("manifest capabilities must not be empty".to_string());
    }

    let mut has_required_buzz_connection = false;
    let mut connected_servers = HashSet::new();
    let mut invoked_servers = HashSet::new();
    for (index, capability) in policy.capabilities.iter().enumerate() {
        let capability_type = capability
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("capabilities[{index}].type must be a string"))?;
        match capability_type {
            "filesystem:read" => validate_filesystem_capability(capability, index, false)?,
            "filesystem:write" => validate_filesystem_capability(capability, index, true)?,
            "terminal:execute" => validate_terminal_capability(capability, index)?,
            "mcp:connect" => {
                validate_mcp_capability(capability, index, false)?;
                let servers = string_array(capability, index, "servers", true)?;
                has_required_buzz_connection |= servers.contains(&BUZZ_DEV_MCP_SERVER);
                connected_servers.extend(servers.into_iter().map(str::to_string));
            }
            "mcp:invoke" => {
                validate_mcp_capability(capability, index, true)?;
                invoked_servers.extend(
                    string_array(capability, index, "servers", true)?
                        .into_iter()
                        .map(str::to_string),
                );
            }
            _ => {
                return Err(format!(
                    "capabilities[{index}] uses unsupported type {capability_type}"
                ));
            }
        }
    }
    if !has_required_buzz_connection {
        return Err(format!(
            "manifest capabilities must include mcp:connect servers: [{BUZZ_DEV_MCP_SERVER}]"
        ));
    }
    let mut missing_connections: Vec<_> = invoked_servers
        .difference(&connected_servers)
        .cloned()
        .collect();
    missing_connections.sort();
    if !missing_connections.is_empty() {
        return Err(format!(
            "mcp:invoke servers require matching mcp:connect grants: {}",
            missing_connections.join(", ")
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> NxtlinqManifestPolicyDraft {
        NxtlinqManifestPolicyDraft {
            name: "review-agent".into(),
            version: "1.0.0".into(),
            scope: vec![NXTLINQ_STRUCTURED_SCOPE.into()],
            aud: vec![NXTLINQ_AUDIENCE.into()],
            capabilities: [
                serde_json::json!({
                    "type": "filesystem:read",
                    "include": ["README.md", "src/**"],
                    "exclude": REQUIRED_SENSITIVE_EXCLUDES
                }),
                serde_json::json!({
                    "type": "mcp:connect",
                    "servers": [BUZZ_DEV_MCP_SERVER]
                }),
            ]
            .into_iter()
            .map(|value| serde_json::from_value(value).unwrap())
            .collect(),
            exp: None,
        }
    }

    fn policy_with_capability(capability: Value) -> NxtlinqManifestPolicyDraft {
        let mut policy = policy();
        policy.capabilities = [
            capability,
            serde_json::json!({
                "type": "mcp:connect",
                "servers": [BUZZ_DEV_MCP_SERVER]
            }),
        ]
        .into_iter()
        .map(|value| serde_json::from_value(value).unwrap())
        .collect();
        policy
    }

    #[test]
    fn conversational_policy_uses_exact_structured_scope_and_gateway_audience() {
        let mut draft = policy();
        draft.scope = vec!["/Users/owner/project".into()];
        assert!(validate_policy(&draft)
            .unwrap_err()
            .contains("scope must be exactly"));

        let mut draft = policy();
        draft.aud.push("another-verifier".into());
        assert!(validate_policy(&draft)
            .unwrap_err()
            .contains("audience must be exactly"));

        let mut draft = policy();
        draft.capabilities.retain(|capability| {
            capability.get("type").and_then(Value::as_str) != Some("mcp:connect")
        });
        assert!(validate_policy(&draft).unwrap_err().contains("mcp:connect"));
    }

    #[test]
    fn filesystem_policy_requires_relative_include_patterns() {
        for pattern in [
            "/Users/owner/project/**",
            r"\\server\share\**",
            r"C:\project\**",
            "src/../.env",
        ] {
            let draft = policy_with_capability(serde_json::json!({
                "type": "filesystem:read",
                "include": [pattern]
            }));
            assert!(validate_policy(&draft).is_err(), "accepted {pattern}");
        }

        let missing = policy_with_capability(serde_json::json!({
            "type": "filesystem:read"
        }));
        assert!(validate_policy(&missing)
            .unwrap_err()
            .contains("include must be a non-empty string array"));

        let wrong_type = policy_with_capability(serde_json::json!({
            "type": "filesystem:read",
            "include": "src/**"
        }));
        assert!(validate_policy(&wrong_type)
            .unwrap_err()
            .contains("include must be a string array"));

        let missing_sensitive_excludes = policy_with_capability(serde_json::json!({
            "type": "filesystem:read",
            "include": ["README.md", "src/**"],
            "exclude": []
        }));
        assert!(validate_policy(&missing_sensitive_excludes)
            .unwrap_err()
            .contains("required sensitive-file exclusions"));
    }

    #[test]
    fn terminal_policy_uses_exact_commands_and_environment_names() {
        let valid = policy_with_capability(serde_json::json!({
            "type": "terminal:execute",
            "commands": ["git status", "npm start"],
            "environment": ["PATH", "BUZZ_AGENT_MODEL"],
            "approvalRequired": false
        }));
        validate_policy(&valid).unwrap();

        let explicit_wrapper = policy_with_capability(serde_json::json!({
            "type": "terminal:execute",
            "commands": ["/bin/bash -lc 'pwd'"],
            "environment": ["PATH"]
        }));
        validate_policy(&explicit_wrapper).unwrap();

        let environment_value = policy_with_capability(serde_json::json!({
            "type": "terminal:execute",
            "commands": ["npm start"],
            "environment": ["API_TOKEN=secret"]
        }));
        assert!(validate_policy(&environment_value)
            .unwrap_err()
            .contains("variable names only"));

        let host_identity = policy_with_capability(serde_json::json!({
            "type": "terminal:execute",
            "commands": ["env"],
            "environment": ["PATH", "BUZZ_PRIVATE_KEY"]
        }));
        assert!(validate_policy(&host_identity)
            .unwrap_err()
            .contains("host identity variable"));

        let scalar_commands = policy_with_capability(serde_json::json!({
            "type": "terminal:execute",
            "commands": "git status"
        }));
        assert!(validate_policy(&scalar_commands)
            .unwrap_err()
            .contains("commands must be a string array"));

        let numeric_approval = policy_with_capability(serde_json::json!({
            "type": "terminal:execute",
            "commands": ["git status"],
            "environment": ["PATH"],
            "approvalRequired": 1
        }));
        assert!(validate_policy(&numeric_approval)
            .unwrap_err()
            .contains("approvalRequired must be a boolean"));

        let unsupported_approval = policy_with_capability(serde_json::json!({
            "type": "terminal:execute",
            "commands": ["git status"],
            "environment": ["PATH"],
            "approvalRequired": true
        }));
        assert!(validate_policy(&unsupported_approval)
            .unwrap_err()
            .contains("unsupported by conversational setup"));
    }

    #[test]
    fn mcp_policy_requires_explicit_unambiguous_selectors() {
        let connect = policy_with_capability(serde_json::json!({
            "type": "mcp:connect",
            "servers": ["customer-mcp"]
        }));
        validate_policy(&connect).unwrap();

        let mut invoke = policy_with_capability(serde_json::json!({
            "type": "mcp:connect",
            "servers": ["customer-mcp"]
        }));
        invoke.capabilities.push(
            serde_json::from_value(serde_json::json!({
                "type": "mcp:invoke",
                "servers": ["customer-mcp"],
                "tools": ["lookup"]
            }))
            .unwrap(),
        );
        validate_policy(&invoke).unwrap();

        let unconnected_invoke = policy_with_capability(serde_json::json!({
            "type": "mcp:invoke",
            "servers": ["customer-mcp"],
            "tools": ["lookup"]
        }));
        assert!(validate_policy(&unconnected_invoke)
            .unwrap_err()
            .contains("matching mcp:connect"));

        let remote_image = policy_with_capability(serde_json::json!({
            "type": "mcp:invoke",
            "servers": [BUZZ_DEV_MCP_SERVER],
            "tools": ["view_image"]
        }));
        validate_policy(&remote_image).unwrap();

        let singular_server = policy_with_capability(serde_json::json!({
            "type": "mcp:connect",
            "server": "customer-mcp"
        }));
        assert!(validate_policy(&singular_server)
            .unwrap_err()
            .contains("unsupported constraint server"));

        let unconstrained_tool = policy_with_capability(serde_json::json!({
            "type": "mcp:invoke",
            "servers": ["customer-mcp"]
        }));
        assert!(validate_policy(&unconstrained_tool)
            .unwrap_err()
            .contains("tools must be a non-empty string array"));

        let cross_product = policy_with_capability(serde_json::json!({
            "type": "mcp:invoke",
            "servers": ["customer-mcp", BUZZ_DEV_MCP_SERVER],
            "tools": ["lookup"]
        }));
        assert!(validate_policy(&cross_product)
            .unwrap_err()
            .contains("exactly one server"));

        let singular_tool = policy_with_capability(serde_json::json!({
            "type": "mcp:invoke",
            "servers": ["customer-mcp"],
            "tool": "lookup"
        }));
        assert!(validate_policy(&singular_tool)
            .unwrap_err()
            .contains("unsupported constraint tool"));
    }

    #[test]
    fn bundled_buzz_tools_use_semantic_capabilities_not_mcp_invoke() {
        for tool in BUNDLED_BUZZ_MCP_TOOLS {
            let draft = policy_with_capability(serde_json::json!({
                "type": "mcp:invoke",
                "servers": [BUZZ_DEV_MCP_SERVER],
                "tools": [tool]
            }));
            assert!(validate_policy(&draft)
                .unwrap_err()
                .contains("filesystem or terminal capabilities"));
        }

        let mut external_same_name = policy_with_capability(serde_json::json!({
            "type": "mcp:connect",
            "servers": ["customer-mcp"]
        }));
        external_same_name.capabilities.push(
            serde_json::from_value(serde_json::json!({
                "type": "mcp:invoke",
                "servers": ["customer-mcp"],
                "tools": ["shell"]
            }))
            .unwrap(),
        );
        validate_policy(&external_same_name).unwrap();
    }
}
