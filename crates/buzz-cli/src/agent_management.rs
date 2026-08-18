//! Owner-reviewed agent draft requests published through Buzz observer frames.

use buzz_core::observer::{encrypt_observer_payload, OBSERVER_FRAME_TELEMETRY};
use nostr::{Event, Keys, PublicKey};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::error::CliError;

const REQUEST_KIND: &str = "agent_management_request";
const MAX_NAME_CHARS: usize = 120;
const MAX_PROMPT_CHARS: usize = 20_000;
const MAX_EXPLANATION_CHARS: usize = 4_000;
const NXTLINQ_AUDIENCE: &str = "nxtlinq-authorization-gateway";
const NXTLINQ_STRUCTURED_SCOPE: &str = "demo:structured-capabilities";
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
const REQUIRED_SENSITIVE_EXCLUDES: &[&str] = &[
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NxtlinqPolicyDraft {
    pub name: String,
    pub version: String,
    pub scope: Vec<String>,
    pub aud: Vec<String>,
    pub capabilities: Vec<serde_json::Map<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exp: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NxtlinqSetupDraft {
    pub channel_id: String,
    pub project_root: String,
    pub explanation: String,
    pub policy: NxtlinqPolicyDraft,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAgentDraft {
    pub channel_id: String,
    pub display_name: String,
    pub system_prompt: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAgentDraft {
    pub channel_id: String,
    pub agent_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub respond_to: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagementRequest<T> {
    #[serde(rename = "type")]
    request_type: &'static str,
    action: &'static str,
    request_id: String,
    request: T,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ObserverEvent<T> {
    seq: u64,
    timestamp: String,
    kind: &'static str,
    agent_index: Option<usize>,
    channel_id: Option<String>,
    session_id: Option<String>,
    turn_id: Option<String>,
    payload: ManagementRequest<T>,
}

#[derive(Debug)]
pub struct BuiltDraftRequest {
    pub event: Event,
    pub request_id: String,
    pub action: &'static str,
}

fn required(value: String, label: &str, max: usize) -> Result<String, CliError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(CliError::Usage(format!("{label} is required")));
    }
    if value.chars().count() > max {
        return Err(CliError::Usage(format!(
            "{label} is too long (max {max} characters)"
        )));
    }
    Ok(value.to_owned())
}

fn optional(value: Option<String>, label: &str) -> Result<Option<String>, CliError> {
    value.map(|value| required(value, label, 300)).transpose()
}

fn capability_error(message: impl Into<String>) -> CliError {
    CliError::Usage(message.into())
}

fn validate_allowed_fields(
    capability: &serde_json::Map<String, serde_json::Value>,
    index: usize,
    allowed: &[&str],
) -> Result<(), CliError> {
    for name in capability.keys() {
        if !allowed.contains(&name.as_str()) {
            return Err(capability_error(format!(
                "capabilities[{index}] contains unsupported constraint {name}"
            )));
        }
    }
    Ok(())
}

fn string_array<'a>(
    capability: &'a serde_json::Map<String, serde_json::Value>,
    index: usize,
    field: &str,
    required: bool,
) -> Result<Vec<&'a str>, CliError> {
    let Some(value) = capability.get(field) else {
        if required {
            return Err(capability_error(format!(
                "capabilities[{index}].{field} must be a non-empty string array"
            )));
        }
        return Ok(Vec::new());
    };
    let values = value.as_array().ok_or_else(|| {
        capability_error(format!(
            "capabilities[{index}].{field} must be a string array"
        ))
    })?;
    if required && values.is_empty() {
        return Err(capability_error(format!(
            "capabilities[{index}].{field} must be a non-empty string array"
        )));
    }
    values
        .iter()
        .enumerate()
        .map(|(value_index, value)| {
            value
                .as_str()
                .filter(|text| !text.trim().is_empty())
                .ok_or_else(|| {
                    capability_error(format!(
                        "capabilities[{index}].{field}[{value_index}] must be a non-empty string"
                    ))
                })
        })
        .collect()
}

fn validate_approval_required(
    capability: &serde_json::Map<String, serde_json::Value>,
    index: usize,
) -> Result<(), CliError> {
    let approval_required = capability.get("approvalRequired");
    if approval_required.is_some_and(|value| !value.is_boolean()) {
        return Err(capability_error(format!(
            "capabilities[{index}].approvalRequired must be a boolean"
        )));
    }
    if approval_required.and_then(serde_json::Value::as_bool) == Some(true) {
        return Err(capability_error(format!(
            "capabilities[{index}].approvalRequired=true is unsupported by conversational setup"
        )));
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
) -> Result<(), CliError> {
    if pattern.contains('\0') {
        return Err(capability_error(format!(
            "capabilities[{index}].{field} contains a NUL byte"
        )));
    }
    if pattern.starts_with('/') || pattern.starts_with('\\') || is_windows_drive_absolute(pattern) {
        return Err(capability_error(format!(
            "capabilities[{index}].{field} must use project-relative patterns, not {pattern}"
        )));
    }
    if pattern
        .split(['/', '\\'])
        .any(|component| component == "..")
    {
        return Err(capability_error(format!(
            "capabilities[{index}].{field} must not contain parent-directory segments"
        )));
    }
    Ok(())
}

fn valid_environment_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn validate_nxtlinq_policy(policy: &NxtlinqPolicyDraft) -> Result<(), CliError> {
    required(policy.name.clone(), "manifest name", 300)?;
    required(policy.version.clone(), "manifest version", 300)?;
    if policy.scope != [NXTLINQ_STRUCTURED_SCOPE] {
        return Err(capability_error(format!(
            "manifest scope must be exactly [{NXTLINQ_STRUCTURED_SCOPE}]"
        )));
    }
    if policy.aud != [NXTLINQ_AUDIENCE] {
        return Err(capability_error(format!(
            "manifest audience must be exactly [{NXTLINQ_AUDIENCE}]"
        )));
    }
    if policy.capabilities.is_empty() {
        return Err(CliError::Usage(
            "manifest capabilities must not be empty".into(),
        ));
    }
    let mut has_required_buzz_connection = false;
    let mut connected_servers = HashSet::new();
    let mut invoked_servers = HashSet::new();
    for (index, capability) in policy.capabilities.iter().enumerate() {
        let capability_type = capability
            .get("type")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                CliError::Usage(format!("capabilities[{index}].type must be a string"))
            })?;
        match capability_type {
            "filesystem:read" | "filesystem:write" => {
                let writable = capability_type == "filesystem:write";
                let allowed: &[&str] = if writable {
                    &["type", "include", "exclude", "approvalRequired"]
                } else {
                    &["type", "include", "exclude"]
                };
                validate_allowed_fields(capability, index, allowed)?;
                for pattern in string_array(capability, index, "include", true)? {
                    validate_project_relative_pattern(pattern, index, "include")?;
                }
                let excludes = string_array(capability, index, "exclude", false)?;
                for pattern in &excludes {
                    validate_project_relative_pattern(pattern, index, "exclude")?;
                }
                if REQUIRED_SENSITIVE_EXCLUDES
                    .iter()
                    .any(|required| !excludes.contains(required))
                {
                    return Err(capability_error(format!(
                        "capabilities[{index}].exclude must contain the required sensitive-file exclusions"
                    )));
                }
                if writable {
                    validate_approval_required(capability, index)?;
                }
            }
            "terminal:execute" => {
                validate_allowed_fields(
                    capability,
                    index,
                    &["type", "commands", "environment", "approvalRequired"],
                )?;
                for command in string_array(capability, index, "commands", true)? {
                    if command.contains('\0') {
                        return Err(capability_error(format!(
                            "capabilities[{index}].commands must not contain NUL bytes"
                        )));
                    }
                }
                let environment = string_array(capability, index, "environment", false)?;
                for name in &environment {
                    if !valid_environment_name(name) {
                        return Err(capability_error(format!(
                            "capabilities[{index}].environment must contain environment variable names only"
                        )));
                    }
                    if FORBIDDEN_TERMINAL_ENVIRONMENT_NAMES.contains(name) {
                        return Err(capability_error(format!(
                            "capabilities[{index}].environment cannot expose host identity variable {name}"
                        )));
                    }
                }
                if !environment.contains(&"PATH") {
                    return Err(capability_error(format!(
                        "capabilities[{index}].environment must include PATH for protected shell execution"
                    )));
                }
                validate_approval_required(capability, index)?;
            }
            "mcp:connect" | "mcp:invoke" => {
                let invoke = capability_type == "mcp:invoke";
                let allowed: &[&str] = if invoke {
                    &["type", "servers", "tools", "approvalRequired"]
                } else {
                    &["type", "servers", "approvalRequired"]
                };
                validate_allowed_fields(capability, index, allowed)?;
                let servers = string_array(capability, index, "servers", true)?;
                if !invoke {
                    has_required_buzz_connection |= servers.contains(&BUZZ_DEV_MCP_SERVER);
                    connected_servers.extend(servers.iter().map(|server| (*server).to_string()));
                }
                if invoke {
                    if servers.len() != 1 {
                        return Err(capability_error(format!(
                            "capabilities[{index}].servers must contain exactly one server for mcp:invoke"
                        )));
                    }
                    invoked_servers.extend(servers.iter().map(|server| (*server).to_string()));
                    let tools = string_array(capability, index, "tools", true)?;
                    if servers.contains(&BUZZ_DEV_MCP_SERVER)
                        && tools
                            .iter()
                            .any(|tool| BUNDLED_BUZZ_MCP_TOOLS.contains(tool))
                    {
                        return Err(capability_error(format!(
                            "capabilities[{index}] must authorize bundled Buzz tools through filesystem or terminal capabilities, not mcp:invoke"
                        )));
                    }
                }
                validate_approval_required(capability, index)?;
            }
            _ => {
                return Err(capability_error(format!(
                    "capabilities[{index}] uses unsupported type {capability_type}"
                )))
            }
        }
    }
    if !has_required_buzz_connection {
        return Err(capability_error(format!(
            "manifest capabilities must include mcp:connect servers: [{BUZZ_DEV_MCP_SERVER}]"
        )));
    }
    let mut missing_connections: Vec<_> = invoked_servers
        .difference(&connected_servers)
        .cloned()
        .collect();
    missing_connections.sort();
    if !missing_connections.is_empty() {
        return Err(capability_error(format!(
            "mcp:invoke servers require matching mcp:connect grants: {}",
            missing_connections.join(", ")
        )));
    }
    Ok(())
}

fn build<T: Serialize>(
    keys: &Keys,
    owner: &PublicKey,
    channel_id: String,
    action: &'static str,
    request: T,
) -> Result<BuiltDraftRequest, CliError> {
    let request_id = uuid::Uuid::new_v4().to_string();
    let payload = ObserverEvent {
        seq: 0,
        timestamp: chrono::Utc::now().to_rfc3339(),
        kind: REQUEST_KIND,
        agent_index: None,
        channel_id: Some(channel_id),
        session_id: None,
        turn_id: None,
        payload: ManagementRequest {
            request_type: REQUEST_KIND,
            action,
            request_id: request_id.clone(),
            request,
        },
    };
    let encrypted = encrypt_observer_payload(keys, owner, &payload)
        .map_err(|error| CliError::Other(format!("could not encrypt draft request: {error}")))?;
    let event = buzz_sdk::build_agent_observer_frame(
        &owner.to_hex(),
        &keys.public_key().to_hex(),
        OBSERVER_FRAME_TELEMETRY,
        &encrypted,
    )
    .map_err(|error| CliError::Other(format!("could not build draft request: {error}")))?
    .sign_with_keys(keys)
    .map_err(|error| CliError::Other(format!("could not sign draft request: {error}")))?;
    Ok(BuiltDraftRequest {
        event,
        request_id,
        action,
    })
}

pub fn build_create(
    keys: &Keys,
    owner: &PublicKey,
    draft: CreateAgentDraft,
) -> Result<BuiltDraftRequest, CliError> {
    let channel_id = required(draft.channel_id, "channel", 128)?;
    uuid::Uuid::parse_str(&channel_id)
        .map_err(|_| CliError::Usage(format!("invalid channel UUID: {channel_id}")))?;
    let request = CreateAgentDraft {
        channel_id: channel_id.clone(),
        display_name: required(draft.display_name, "display name", MAX_NAME_CHARS)?,
        system_prompt: required(draft.system_prompt, "system prompt", MAX_PROMPT_CHARS)?,
    };
    build(keys, owner, channel_id, "create", request)
}

pub fn build_update(
    keys: &Keys,
    owner: &PublicKey,
    draft: UpdateAgentDraft,
) -> Result<BuiltDraftRequest, CliError> {
    let channel_id = required(draft.channel_id, "channel", 128)?;
    uuid::Uuid::parse_str(&channel_id)
        .map_err(|_| CliError::Usage(format!("invalid channel UUID: {channel_id}")))?;
    let respond_to = optional(draft.respond_to, "respond-to")?;
    if respond_to
        .as_deref()
        .is_some_and(|value| value != "owner-only" && value != "anyone")
    {
        return Err(CliError::Usage(
            "respond-to must be owner-only or anyone".into(),
        ));
    }
    let request = UpdateAgentDraft {
        channel_id: channel_id.clone(),
        agent_name: required(draft.agent_name, "agent name", MAX_NAME_CHARS)?,
        display_name: optional(draft.display_name, "display name")?,
        system_prompt: draft
            .system_prompt
            .map(|value| required(value, "system prompt", MAX_PROMPT_CHARS))
            .transpose()?,
        runtime: optional(draft.runtime, "runtime")?,
        provider: optional(draft.provider, "provider")?,
        model: optional(draft.model, "model")?,
        respond_to,
    };
    if request.display_name.is_none()
        && request.system_prompt.is_none()
        && request.runtime.is_none()
        && request.provider.is_none()
        && request.model.is_none()
        && request.respond_to.is_none()
    {
        return Err(CliError::Usage(
            "include at least one field to update".into(),
        ));
    }
    build(keys, owner, channel_id, "update", request)
}

pub fn build_nxtlinq_setup(
    keys: &Keys,
    owner: &PublicKey,
    draft: NxtlinqSetupDraft,
) -> Result<BuiltDraftRequest, CliError> {
    validate_nxtlinq_policy(&draft.policy)?;
    let channel_id = required(draft.channel_id, "channel", 128)?;
    uuid::Uuid::parse_str(&channel_id)
        .map_err(|_| CliError::Usage(format!("invalid channel UUID: {channel_id}")))?;
    let request = NxtlinqSetupDraft {
        channel_id: channel_id.clone(),
        project_root: required(draft.project_root, "project root", 4_096)?,
        explanation: required(
            draft.explanation,
            "policy explanation",
            MAX_EXPLANATION_CHARS,
        )?,
        policy: draft.policy,
    };
    build(keys, owner, channel_id, "nxtlinq_setup", request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::observer::{decrypt_observer_payload, OBSERVER_AGENT_TAG, OBSERVER_FRAME_TAG};

    const CHANNEL: &str = "7c07e659-3610-42f4-9a5e-1e9973c09da9";

    #[test]
    fn create_is_owner_encrypted_and_matches_desktop_contract() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let built = build_create(
            &agent,
            &owner.public_key(),
            CreateAgentDraft {
                channel_id: CHANNEL.into(),
                display_name: "Research helper".into(),
                system_prompt: "Find sources.".into(),
            },
        )
        .unwrap();

        assert_eq!(built.event.kind.as_u16(), 24_200);
        let tags: Vec<Vec<String>> = built
            .event
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect();
        assert!(tags
            .iter()
            .any(|tag| tag == &["p", &owner.public_key().to_hex()]));
        assert!(tags
            .iter()
            .any(|tag| tag == &[OBSERVER_AGENT_TAG, &agent.public_key().to_hex()]));
        assert!(tags
            .iter()
            .any(|tag| tag == &[OBSERVER_FRAME_TAG, OBSERVER_FRAME_TELEMETRY]));
        assert!(!tags
            .iter()
            .any(|tag| tag.first().map(String::as_str) == Some("h")));

        let payload: serde_json::Value = decrypt_observer_payload(&owner, &built.event).unwrap();
        assert_eq!(payload["kind"], REQUEST_KIND);
        assert_eq!(payload["channelId"], CHANNEL);
        assert_eq!(payload["payload"]["type"], REQUEST_KIND);
        assert_eq!(payload["payload"]["action"], "create");
        assert_eq!(
            payload["payload"]["request"]["displayName"],
            "Research helper"
        );
        assert!(payload["payload"]["request"].get("runtime").is_none());
        assert!(payload["payload"]["request"].get("respondTo").is_none());
    }

    #[test]
    fn update_requires_a_change() {
        let error = build_update(
            &Keys::generate(),
            &Keys::generate().public_key(),
            UpdateAgentDraft {
                channel_id: CHANNEL.into(),
                agent_name: "Scout".into(),
                display_name: None,
                system_prompt: None,
                runtime: None,
                provider: None,
                model: None,
                respond_to: None,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("at least one field"));
    }

    #[test]
    fn create_rejects_invalid_channel() {
        let error = build_create(
            &Keys::generate(),
            &Keys::generate().public_key(),
            CreateAgentDraft {
                channel_id: "general".into(),
                display_name: "Scout".into(),
                system_prompt: "Help".into(),
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("invalid channel UUID"));
    }

    #[test]
    fn nxtlinq_setup_is_owner_encrypted_and_contains_no_private_key() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let built = build_nxtlinq_setup(
            &agent,
            &owner.public_key(),
            NxtlinqSetupDraft {
                channel_id: CHANNEL.into(),
                project_root: "/workspace/project".into(),
                explanation: "Read source, but exclude secrets.".into(),
                policy: NxtlinqPolicyDraft {
                    name: "review-agent".into(),
                    version: "1.0.0".into(),
                    scope: vec!["demo:structured-capabilities".into()],
                    aud: vec!["nxtlinq-authorization-gateway".into()],
                    capabilities: [
                        serde_json::json!({
                            "type": "filesystem:read",
                            "include": ["src/**"],
                            "exclude": REQUIRED_SENSITIVE_EXCLUDES
                        }),
                        serde_json::json!({
                            "type": "mcp:connect",
                            "servers": ["buzz-dev-mcp"]
                        }),
                    ]
                    .into_iter()
                    .map(|value| serde_json::from_value(value).unwrap())
                    .collect(),
                    exp: None,
                },
            },
        )
        .unwrap();
        let payload: serde_json::Value = decrypt_observer_payload(&owner, &built.event).unwrap();
        assert_eq!(payload["payload"]["action"], "nxtlinq_setup");
        assert!(payload.to_string().contains("filesystem:read"));
        assert!(payload["payload"]["request"]["policy"].get("exp").is_none());
        assert!(!payload
            .to_string()
            .to_ascii_lowercase()
            .contains("privatekey"));
    }

    #[test]
    fn nxtlinq_setup_rejects_constraints_desktop_would_drop() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let error = build_nxtlinq_setup(
            &agent,
            &owner.public_key(),
            NxtlinqSetupDraft {
                channel_id: CHANNEL.into(),
                project_root: "/workspace/project".into(),
                explanation: "Run the application without network access.".into(),
                policy: NxtlinqPolicyDraft {
                    name: "review-agent".into(),
                    version: "1.0.0".into(),
                    scope: vec!["demo:structured-capabilities".into()],
                    aud: vec!["nxtlinq-authorization-gateway".into()],
                    capabilities: vec![serde_json::from_value(serde_json::json!({
                        "type": "terminal:execute",
                        "commands": ["npm start"],
                        "network": false
                    }))
                    .unwrap()],
                    exp: None,
                },
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("unsupported constraint network"));
    }

    fn policy_with_capability(capability: serde_json::Value) -> NxtlinqPolicyDraft {
        NxtlinqPolicyDraft {
            name: "review-agent".into(),
            version: "1.0.0".into(),
            scope: vec![NXTLINQ_STRUCTURED_SCOPE.into()],
            aud: vec![NXTLINQ_AUDIENCE.into()],
            capabilities: [
                capability,
                serde_json::json!({
                    "type": "mcp:connect",
                    "servers": ["buzz-dev-mcp"]
                }),
            ]
            .into_iter()
            .map(|value| serde_json::from_value(value).unwrap())
            .collect(),
            exp: None,
        }
    }

    #[test]
    fn nxtlinq_setup_rejects_broad_or_nonportable_filesystem_constraints() {
        for capability in [
            serde_json::json!({ "type": "filesystem:read" }),
            serde_json::json!({ "type": "filesystem:read", "include": "src/**" }),
            serde_json::json!({ "type": "filesystem:read", "include": ["/project/**"] }),
            serde_json::json!({ "type": "filesystem:read", "include": [r"C:\project\**"] }),
            serde_json::json!({ "type": "filesystem:read", "include": ["src/../.env"] }),
            serde_json::json!({ "type": "filesystem:read", "include": ["src/**"], "exclude": [] }),
        ] {
            assert!(validate_nxtlinq_policy(&policy_with_capability(capability)).is_err());
        }
    }

    #[test]
    fn nxtlinq_setup_requires_exact_commands_and_environment_names() {
        validate_nxtlinq_policy(&policy_with_capability(serde_json::json!({
            "type": "terminal:execute",
            "commands": ["git status"],
            "environment": ["PATH"]
        })))
        .unwrap();

        for capability in [
            serde_json::json!({ "type": "terminal:execute" }),
            serde_json::json!({ "type": "terminal:execute", "commands": "git status" }),
            serde_json::json!({
                "type": "terminal:execute",
                "commands": ["npm start"],
                "environment": ["TOKEN=secret"]
            }),
            serde_json::json!({
                "type": "terminal:execute",
                "commands": ["git status"],
                "environment": ["PATH"],
                "approvalRequired": true
            }),
            serde_json::json!({
                "type": "terminal:execute",
                "commands": ["git status"],
                "environment": ["PATH", "BUZZ_PRIVATE_KEY"]
            }),
        ] {
            assert!(validate_nxtlinq_policy(&policy_with_capability(capability)).is_err());
        }
    }

    #[test]
    fn nxtlinq_setup_requires_explicit_canonical_mcp_selectors() {
        validate_nxtlinq_policy(&policy_with_capability(serde_json::json!({
            "type": "mcp:connect",
            "servers": ["buzz-dev-mcp"]
        })))
        .unwrap();
        let mut external = policy_with_capability(serde_json::json!({
            "type": "mcp:connect",
            "servers": ["customer-mcp"]
        }));
        external.capabilities.push(
            serde_json::from_value(serde_json::json!({
                "type": "mcp:invoke",
                "servers": ["customer-mcp"],
                "tools": ["lookup"]
            }))
            .unwrap(),
        );
        validate_nxtlinq_policy(&external).unwrap();
        assert!(
            validate_nxtlinq_policy(&policy_with_capability(serde_json::json!({
                "type": "mcp:invoke",
                "servers": ["customer-mcp"],
                "tools": ["lookup"]
            })))
            .unwrap_err()
            .to_string()
            .contains("matching mcp:connect")
        );
        validate_nxtlinq_policy(&policy_with_capability(serde_json::json!({
            "type": "mcp:invoke",
            "servers": ["buzz-dev-mcp"],
            "tools": ["view_image"]
        })))
        .unwrap();

        for capability in [
            serde_json::json!({ "type": "mcp:connect", "server": "buzz-dev-mcp" }),
            serde_json::json!({ "type": "mcp:invoke", "servers": ["customer-mcp"] }),
            serde_json::json!({
                "type": "mcp:invoke",
                "servers": ["customer-mcp", "buzz-dev-mcp"],
                "tools": ["lookup"]
            }),
            serde_json::json!({
                "type": "mcp:invoke",
                "servers": ["buzz-dev-mcp"],
                "tools": ["shell"]
            }),
        ] {
            assert!(validate_nxtlinq_policy(&policy_with_capability(capability)).is_err());
        }
    }

    #[test]
    fn nxtlinq_setup_rejects_legacy_authorizing_scope() {
        let mut policy = policy_with_capability(serde_json::json!({
            "type": "filesystem:read",
            "include": ["README.md"]
        }));
        policy.scope = vec!["tool:terminal:execute".into()];
        assert!(validate_nxtlinq_policy(&policy)
            .unwrap_err()
            .to_string()
            .contains("scope must be exactly"));
    }

    #[test]
    fn nxtlinq_setup_requires_the_buzz_mcp_session_connection() {
        let policy = NxtlinqPolicyDraft {
            name: "review-agent".into(),
            version: "1.0.0".into(),
            scope: vec![NXTLINQ_STRUCTURED_SCOPE.into()],
            aud: vec![NXTLINQ_AUDIENCE.into()],
            capabilities: vec![serde_json::from_value(serde_json::json!({
                "type": "filesystem:read",
                "include": ["README.md"],
                "exclude": REQUIRED_SENSITIVE_EXCLUDES
            }))
            .unwrap()],
            exp: None,
        };
        assert!(validate_nxtlinq_policy(&policy)
            .unwrap_err()
            .to_string()
            .contains("mcp:connect"));
    }
}
