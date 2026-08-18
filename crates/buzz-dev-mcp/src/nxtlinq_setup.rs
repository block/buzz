use crate::shell::SharedState;
use rmcp::model::{CallToolResult, Content};
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const SUBMIT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_EXPLANATION_BYTES: usize = 16 * 1024;
const TRUSTED_DEV_MCP_SERVER: &str = "buzz-dev-mcp";
const CONTEXT_CHANNEL_ENV: &str = "BUZZ_CONTEXT_CHANNEL_ID";
const FORBIDDEN_TERMINAL_ENVIRONMENT_NAMES: &[&str] =
    &["BUZZ_PRIVATE_KEY", "NOSTR_PRIVATE_KEY", "BUZZ_AUTH_TAG"];
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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NxtlinqSetupParams {
    /// Current Buzz channel UUID from the conversation context.
    pub channel: String,
    /// Exact absolute project path explicitly supplied by the owner in the current request.
    /// Never substitute the MCP process cwd or the Agent's default workspace.
    pub owner_project_root: String,
    /// Human-readable explanation of the proposed access ceiling.
    pub explanation: String,
    /// Policy-only Nxtlinq manifest object. Put name, version, scope, aud,
    /// capabilities, and optional exp inside this object, never at the setup
    /// envelope's top level. Never include private/signing keys.
    pub policy: NxtlinqPolicyParams,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NxtlinqPolicyParams {
    /// Human-readable manifest name, for example "customer-project-agent".
    pub name: String,
    /// Manifest version, normally "1.0.0" for a new project.
    pub version: String,
    /// Authorization scope. Conversational Buzz setup uses
    /// ["demo:structured-capabilities"].
    pub scope: Vec<String>,
    /// Intended verifier audience. Must contain
    /// "nxtlinq-authorization-gateway".
    pub aud: Vec<String>,
    /// Narrow capability proposals reviewed and editable by the owner.
    pub capabilities: Vec<NxtlinqCapabilityParams>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exp: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum NxtlinqCapabilityParams {
    #[serde(rename = "filesystem:read")]
    FilesystemRead {
        /// Project-relative file globs. Never prefix owner_project_root.
        include: Vec<String>,
        /// Project-relative deny globs. Sensitive defaults are added automatically.
        #[serde(default)]
        exclude: Vec<String>,
    },
    #[serde(rename = "filesystem:write")]
    FilesystemWrite {
        /// Project-relative file globs. Write does not imply read.
        include: Vec<String>,
        #[serde(default)]
        exclude: Vec<String>,
        #[serde(
            rename = "approvalRequired",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        approval_required: Option<bool>,
    },
    #[serde(rename = "terminal:execute")]
    TerminalExecute {
        /// Exact raw shell tool command strings. Filesystem excludes do not constrain them.
        commands: Vec<String>,
        /// Environment variable names only, never NAME=value pairs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        environment: Option<Vec<String>>,
        #[serde(
            rename = "approvalRequired",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        approval_required: Option<bool>,
    },
    #[serde(rename = "mcp:connect")]
    McpConnect {
        /// Exact MCP server names. Buzz Agent sessions require buzz-dev-mcp.
        servers: Vec<String>,
        #[serde(
            rename = "approvalRequired",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        approval_required: Option<bool>,
    },
    #[serde(rename = "mcp:invoke")]
    McpInvoke {
        /// Exact MCP server names.
        servers: Vec<String>,
        /// Exact tool names. Bundled file/shell tools use filesystem/terminal capabilities instead.
        tools: Vec<String>,
        #[serde(
            rename = "approvalRequired",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        approval_required: Option<bool>,
    },
}

pub async fn run(state: &SharedState, p: NxtlinqSetupParams) -> Result<CallToolResult, ErrorData> {
    validate(&p)?;
    validate_context_channel(
        &p.channel,
        std::env::var(CONTEXT_CHANNEL_ENV).ok().as_deref(),
    )?;
    let project_root = validate_project_root(&p.owner_project_root)?;
    let policy = normalize_policy(p.policy, &project_root)?;
    let policy = serde_json::to_vec(&policy).map_err(|error| {
        ErrorData::internal_error(format!("failed to serialize Nxtlinq policy: {error}"), None)
    })?;

    // Use the session-owned multicall entrypoint directly. This deliberately
    // bypasses shell parsing and ambient PATH, where an older installed Buzz
    // CLI could otherwise shadow the CLI bundled with this MCP server.
    let mut command = Command::new(&state.shim.buzz_path);
    command
        .arg("agents")
        .arg("nxtlinq-setup")
        .arg("--channel")
        .arg(&p.channel)
        .arg("--project-root")
        .arg(&project_root)
        .arg("--policy")
        .arg("-")
        .arg("--explanation")
        .arg(&p.explanation)
        .current_dir(&state.cwd)
        .env("PATH", &state.shim.path_env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    crate::configure_no_window_async(&mut command);

    let mut child = command.spawn().map_err(|error| {
        ErrorData::internal_error(
            format!("failed to start fixed Nxtlinq setup submitter: {error}"),
            None,
        )
    })?;
    let mut stdin = child.stdin.take().ok_or_else(|| {
        ErrorData::internal_error("Nxtlinq setup submitter stdin was unavailable", None)
    })?;
    stdin.write_all(&policy).await.map_err(|error| {
        ErrorData::internal_error(format!("failed to write Nxtlinq policy: {error}"), None)
    })?;
    drop(stdin);

    let output = match tokio::time::timeout(SUBMIT_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "Nxtlinq setup submission failed: {error}"
            ))]));
        }
        Err(_) => {
            return Ok(CallToolResult::error(vec![Content::text(
                "Nxtlinq setup submission timed out after 30 seconds",
            )]));
        }
    };

    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        return Ok(CallToolResult::error(vec![Content::text(format!(
            "Nxtlinq setup draft was rejected (exit {}): {}",
            output.status.code().unwrap_or(-1),
            detail.trim()
        ))]));
    }
    let detail = String::from_utf8_lossy(&output.stdout);
    Ok(CallToolResult::success(vec![Content::text(
        detail.trim().to_owned(),
    )]))
}

fn policy_error(message: impl Into<String>) -> ErrorData {
    ErrorData::invalid_params(message.into(), None)
}

fn is_portable_absolute(pattern: &str) -> bool {
    let normalized = pattern.replace('\\', "/");
    normalized.starts_with('/')
        || normalized
            .as_bytes()
            .get(1..3)
            .is_some_and(|bytes| bytes[0] == b':' && bytes[1] == b'/')
}

fn valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        && chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn require_nonempty(values: &[String], label: &str) -> Result<(), ErrorData> {
    if values.is_empty() || values.iter().any(|value| value.trim().is_empty()) {
        return Err(policy_error(format!(
            "{label} must be a non-empty array of non-empty strings"
        )));
    }
    Ok(())
}

fn normalize_policy_pattern(pattern: &str, project_root: &Path) -> Result<String, ErrorData> {
    let pattern = pattern.trim();
    if pattern.is_empty() || pattern.contains('\0') {
        return Err(policy_error(
            "policy path pattern must be non-empty and contain no NUL",
        ));
    }
    let path = Path::new(pattern);
    let relative = if path.is_absolute() {
        path.strip_prefix(project_root).map_err(|_| {
            policy_error(format!(
                "absolute policy path is outside owner_project_root: {pattern}"
            ))
        })?
    } else if is_portable_absolute(pattern) {
        return Err(policy_error(format!(
            "policy path must be relative to owner_project_root: {pattern}"
        )));
    } else {
        path
    };
    if relative
        .components()
        .any(|component| component == std::path::Component::ParentDir)
    {
        return Err(policy_error(format!(
            "policy path must not contain parent traversal: {pattern}"
        )));
    }
    let normalized = relative.to_string_lossy().replace('\\', "/");
    if normalized.split('/').any(|component| component == "..") {
        return Err(policy_error(format!(
            "policy path must not contain parent traversal: {pattern}"
        )));
    }
    Ok(if normalized.is_empty() {
        ".".to_string()
    } else {
        normalized
    })
}

fn normalize_patterns(
    patterns: &mut [String],
    project_root: &Path,
    label: &str,
) -> Result<(), ErrorData> {
    require_nonempty(patterns, label)?;
    for pattern in patterns {
        *pattern = normalize_policy_pattern(pattern, project_root)?;
    }
    Ok(())
}

fn normalize_policy(
    mut policy: NxtlinqPolicyParams,
    project_root: &Path,
) -> Result<NxtlinqPolicyParams, ErrorData> {
    let project_scope = format!("project:{}", project_root.display());
    for scope in &mut policy.scope {
        if scope == &project_scope {
            *scope = "demo:structured-capabilities".to_string();
        }
    }

    if policy.scope != ["demo:structured-capabilities"] {
        return Err(policy_error(
            "scope must be exactly [demo:structured-capabilities]; capability grants belong in capabilities",
        ));
    }
    if policy.aud != ["nxtlinq-authorization-gateway"] {
        return Err(policy_error(
            "aud must be exactly [nxtlinq-authorization-gateway]",
        ));
    }

    let mut has_buzz_connect = false;
    let mut connected_servers = HashSet::new();
    let mut invoked_servers = HashSet::new();
    for (index, capability) in policy.capabilities.iter_mut().enumerate() {
        let approval_required = match capability {
            NxtlinqCapabilityParams::FilesystemWrite {
                approval_required, ..
            }
            | NxtlinqCapabilityParams::TerminalExecute {
                approval_required, ..
            }
            | NxtlinqCapabilityParams::McpConnect {
                approval_required, ..
            }
            | NxtlinqCapabilityParams::McpInvoke {
                approval_required, ..
            } => *approval_required,
            NxtlinqCapabilityParams::FilesystemRead { .. } => None,
        };
        if approval_required == Some(true) {
            return Err(policy_error(format!(
                "capabilities[{index}].approvalRequired=true is unsupported by conversational setup"
            )));
        }
        match capability {
            NxtlinqCapabilityParams::FilesystemRead { include, exclude }
            | NxtlinqCapabilityParams::FilesystemWrite {
                include, exclude, ..
            } => {
                normalize_patterns(
                    include,
                    project_root,
                    &format!("capabilities[{index}].include"),
                )?;
                for pattern in exclude.iter_mut() {
                    *pattern = normalize_policy_pattern(pattern, project_root)?;
                }
                for required in REQUIRED_SENSITIVE_EXCLUDES {
                    if !exclude.iter().any(|value| value == required) {
                        exclude.push((*required).to_string());
                    }
                }
            }
            NxtlinqCapabilityParams::TerminalExecute {
                commands,
                environment,
                ..
            } => {
                require_nonempty(commands, &format!("capabilities[{index}].commands"))?;
                let environment = environment.get_or_insert_with(Vec::new);
                if environment.iter().any(|name| !valid_env_name(name)) {
                    return Err(policy_error(format!(
                        "capabilities[{index}].environment accepts variable names only, never NAME=value"
                    )));
                }
                if let Some(name) = environment
                    .iter()
                    .find(|name| FORBIDDEN_TERMINAL_ENVIRONMENT_NAMES.contains(&name.as_str()))
                {
                    return Err(policy_error(format!(
                        "capabilities[{index}].environment cannot expose host identity variable {name}"
                    )));
                }
                if !environment.iter().any(|name| name == "PATH") {
                    environment.push("PATH".to_string());
                }
            }
            NxtlinqCapabilityParams::McpConnect { servers, .. } => {
                require_nonempty(servers, &format!("capabilities[{index}].servers"))?;
                has_buzz_connect |= servers
                    .iter()
                    .any(|server| server == TRUSTED_DEV_MCP_SERVER);
                connected_servers.extend(servers.iter().cloned());
            }
            NxtlinqCapabilityParams::McpInvoke { servers, tools, .. } => {
                require_nonempty(servers, &format!("capabilities[{index}].servers"))?;
                if servers.len() != 1 {
                    return Err(policy_error(format!(
                        "capabilities[{index}].servers must contain exactly one server for mcp:invoke"
                    )));
                }
                require_nonempty(tools, &format!("capabilities[{index}].tools"))?;
                invoked_servers.extend(servers.iter().cloned());
                if servers
                    .iter()
                    .any(|server| server == TRUSTED_DEV_MCP_SERVER)
                    && tools.iter().any(|tool| {
                        matches!(
                            tool.as_str(),
                            "read_file"
                                | "str_replace"
                                | "shell"
                                | "buzz_message_send"
                                | "nxtlinq_setup"
                                | "todo"
                                | "_Stop"
                                | "_PostCompact"
                        )
                    })
                {
                    return Err(policy_error(format!(
                        "capabilities[{index}] must authorize bundled file/shell tools with filesystem/terminal capabilities, not mcp:invoke"
                    )));
                }
            }
        }
    }
    if !has_buzz_connect {
        policy
            .capabilities
            .push(NxtlinqCapabilityParams::McpConnect {
                servers: vec![TRUSTED_DEV_MCP_SERVER.to_string()],
                approval_required: None,
            });
        connected_servers.insert(TRUSTED_DEV_MCP_SERVER.to_string());
    }
    let mut missing_connections: Vec<_> = invoked_servers
        .difference(&connected_servers)
        .cloned()
        .collect();
    missing_connections.sort();
    if !missing_connections.is_empty() {
        return Err(policy_error(format!(
            "mcp:invoke servers require matching mcp:connect grants: {}",
            missing_connections.join(", ")
        )));
    }
    Ok(policy)
}

fn validate(p: &NxtlinqSetupParams) -> Result<(), ErrorData> {
    if p.channel.is_empty() || p.channel.len() > 128 || p.channel.starts_with('-') {
        return Err(ErrorData::invalid_params(
            "invalid channel identifier",
            None,
        ));
    }
    if !Path::new(&p.owner_project_root).is_absolute() {
        return Err(ErrorData::invalid_params(
            "owner_project_root must be the exact absolute path supplied by the owner",
            None,
        ));
    }
    if p.explanation.is_empty() || p.explanation.len() > MAX_EXPLANATION_BYTES {
        return Err(ErrorData::invalid_params(
            format!("explanation must be 1..={MAX_EXPLANATION_BYTES} bytes"),
            None,
        ));
    }
    Ok(())
}

fn validate_context_channel(requested: &str, expected: Option<&str>) -> Result<(), ErrorData> {
    let Some(expected) = expected.filter(|value| !value.is_empty()) else {
        return Err(policy_error(
            "Nxtlinq setup requires a host-bound channel context",
        ));
    };
    if requested != expected {
        return Err(policy_error(
            "Nxtlinq setup channel does not match the host-bound conversation channel",
        ));
    }
    Ok(())
}

/// Validate only the owner-selected project root. Attest initialization is an
/// owner-reviewed Desktop step and is deliberately not a submission
/// prerequisite; this control-plane tool never reads manifest or key contents.
fn validate_project_root(project_root: &str) -> Result<PathBuf, ErrorData> {
    let requested = Path::new(project_root);
    let metadata = std::fs::symlink_metadata(requested).map_err(|error| {
        ErrorData::invalid_params(
            format!(
                "owner_project_root is not accessible: {} ({error}). If the owner supplied another absolute project path in the current request, retry once with that exact path; otherwise ask for it",
                requested.display()
            ),
            None,
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ErrorData::invalid_params(
            format!(
                "owner_project_root must be a real directory, not a symlink: {}. Retry once with the exact absolute path supplied by the owner",
                requested.display()
            ),
            None,
        ));
    }
    std::fs::canonicalize(requested).map_err(|error| {
        ErrorData::invalid_params(
            format!(
                "owner_project_root cannot be resolved: {} ({error})",
                requested.display()
            ),
            None,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> NxtlinqSetupParams {
        NxtlinqSetupParams {
            channel: "5a5f3a27-7f13-4a12-a728-8dd6c3f42032".into(),
            owner_project_root: "/workspace/customer-project".into(),
            explanation: "Read ordinary project source only".into(),
            policy: NxtlinqPolicyParams {
                name: "customer-project".into(),
                version: "1.0.0".into(),
                scope: vec!["demo:structured-capabilities".into()],
                aud: vec!["nxtlinq-authorization-gateway".into()],
                capabilities: vec![NxtlinqCapabilityParams::FilesystemRead {
                    include: vec!["README.md".into(), "src/**".into()],
                    exclude: vec![".env".into(), "nxtlinq/**".into()],
                }],
                exp: None,
            },
        }
    }

    #[test]
    fn rejects_relative_project_root() {
        let mut p = params();
        p.owner_project_root = "customer-project".into();
        assert!(validate(&p).is_err());
    }

    #[test]
    fn setup_channel_is_bound_by_the_host() {
        assert!(validate_context_channel("channel-a", Some("channel-a")).is_ok());
        assert!(validate_context_channel("channel-b", Some("channel-a")).is_err());
        assert!(validate_context_channel("channel-a", None).is_err());
    }

    #[test]
    fn policy_serializes_as_cli_input() {
        let value = serde_json::to_value(params().policy).expect("policy JSON");
        assert_eq!(value["capabilities"][0]["type"], "filesystem:read");
        assert!(value.get("exp").is_none());
    }

    #[test]
    fn setup_and_policy_reject_unknown_fields() {
        let setup = serde_json::json!({
            "channel": "5a5f3a27-7f13-4a12-a728-8dd6c3f42032",
            "owner_project_root": "/workspace/customer-project",
            "explanation": "Read ordinary project source only",
            "policy": {
                "name": "customer-project",
                "version": "1.0.0",
                "scope": ["demo:structured-capabilities"],
                "aud": ["nxtlinq-authorization-gateway"],
                "capabilities": [],
                "unsupportedConstraint": true
            }
        });
        let error = serde_json::from_value::<NxtlinqSetupParams>(setup)
            .expect_err("unknown policy fields must fail closed");
        assert!(error.to_string().contains("unsupportedConstraint"));

        let setup = serde_json::json!({
            "channel": "5a5f3a27-7f13-4a12-a728-8dd6c3f42032",
            "owner_project_root": "/workspace/customer-project",
            "explanation": "Read ordinary project source only",
            "policy": {
                "name": "customer-project",
                "version": "1.0.0",
                "scope": ["demo:structured-capabilities"],
                "aud": ["nxtlinq-authorization-gateway"],
                "capabilities": []
            },
            "unsupportedEnvelopeField": true
        });
        let error = serde_json::from_value::<NxtlinqSetupParams>(setup)
            .expect_err("unknown setup fields must fail closed");
        assert!(error.to_string().contains("unsupportedEnvelopeField"));
    }

    #[test]
    fn normalizes_owner_absolute_patterns_and_adds_sensitive_excludes() {
        let project = tempfile::tempdir().expect("tempdir");
        let mut policy = params().policy;
        policy.capabilities = vec![NxtlinqCapabilityParams::FilesystemRead {
            include: vec![format!("{}/**", project.path().display())],
            exclude: vec![format!("{}/.env", project.path().display())],
        }];
        let normalized = normalize_policy(policy, project.path()).expect("normalized policy");
        let value = serde_json::to_value(&normalized).expect("policy JSON");
        let capability = &value["capabilities"][0];
        assert_eq!(normalized.scope, ["demo:structured-capabilities"]);
        assert_eq!(capability["include"], serde_json::json!(["**"]));
        assert!(capability["exclude"]
            .as_array()
            .expect("excludes")
            .iter()
            .any(|value| value == "nxtlinq/**"));
        assert!(capability["exclude"]
            .as_array()
            .expect("excludes")
            .iter()
            .any(|value| value == "**/.env*"));
        for required in [".npmrc", "**/.netrc", "credentials", "**/.aws/**"] {
            assert!(capability["exclude"]
                .as_array()
                .expect("excludes")
                .iter()
                .any(|value| value == required));
        }
    }

    #[test]
    fn rejects_absolute_policy_paths_outside_owner_project() {
        let project = tempfile::tempdir().expect("tempdir");
        let mut policy = params().policy;
        policy.capabilities[0] = NxtlinqCapabilityParams::FilesystemRead {
            include: vec!["/another/project/**".into()],
            exclude: Vec::new(),
        };
        assert!(normalize_policy(policy, project.path())
            .expect_err("outside path must fail")
            .message
            .contains("outside owner_project_root"));
    }

    #[test]
    fn adds_required_buzz_mcp_connection_without_granting_invocations() {
        let project = tempfile::tempdir().expect("tempdir");
        let normalized = normalize_policy(params().policy, project.path()).expect("normalized");
        let value = serde_json::to_value(normalized).expect("policy JSON");
        assert!(value["capabilities"]
            .as_array()
            .expect("capabilities")
            .iter()
            .any(|capability| capability
                == &serde_json::json!({
                    "type": "mcp:connect",
                    "servers": ["buzz-dev-mcp"]
                })));
        assert!(!value.to_string().contains("mcp:invoke"));
        assert!(!value.to_string().contains("terminal:execute"));
    }

    #[test]
    fn rejects_environment_assignments_and_bundled_mcp_semantic_tools() {
        let project = tempfile::tempdir().expect("tempdir");
        let mut policy = params().policy;
        policy
            .capabilities
            .push(NxtlinqCapabilityParams::TerminalExecute {
                commands: vec!["npm start".into()],
                environment: Some(vec!["TOKEN=secret".into()]),
                approval_required: None,
            });
        assert!(normalize_policy(policy, project.path())
            .expect_err("assignment must fail")
            .message
            .contains("variable names only"));

        let mut policy = params().policy;
        policy
            .capabilities
            .push(NxtlinqCapabilityParams::TerminalExecute {
                commands: vec!["env".into()],
                environment: Some(vec!["BUZZ_PRIVATE_KEY".into()]),
                approval_required: None,
            });
        assert!(normalize_policy(policy, project.path())
            .expect_err("host identity must remain unavailable")
            .message
            .contains("host identity variable"));

        let mut policy = params().policy;
        policy
            .capabilities
            .push(NxtlinqCapabilityParams::McpInvoke {
                servers: vec!["buzz-dev-mcp".into()],
                tools: vec!["shell".into()],
                approval_required: None,
            });
        assert!(normalize_policy(policy, project.path())
            .expect_err("semantic tool must fail")
            .message
            .contains("filesystem/terminal"));
    }

    #[test]
    fn terminal_policy_adds_path_and_rejects_unavailable_interactive_approval() {
        let project = tempfile::tempdir().expect("tempdir");
        let mut policy = params().policy;
        policy
            .capabilities
            .push(NxtlinqCapabilityParams::TerminalExecute {
                commands: vec!["git status".into()],
                environment: None,
                approval_required: None,
            });
        let normalized = normalize_policy(policy, project.path()).expect("normalized");
        let value = serde_json::to_value(normalized).expect("policy JSON");
        let terminal = value["capabilities"]
            .as_array()
            .expect("capabilities")
            .iter()
            .find(|capability| capability["type"] == "terminal:execute")
            .expect("terminal capability");
        assert_eq!(terminal["environment"], serde_json::json!(["PATH"]));

        let mut policy = params().policy;
        policy
            .capabilities
            .push(NxtlinqCapabilityParams::McpConnect {
                servers: vec!["customer-mcp".into()],
                approval_required: Some(true),
            });
        assert!(normalize_policy(policy, project.path())
            .expect_err("interactive approval is unavailable")
            .message
            .contains("unsupported by conversational setup"));
    }

    #[test]
    fn mcp_invoke_does_not_create_a_server_tool_cross_product() {
        let project = tempfile::tempdir().expect("tempdir");
        let mut policy = params().policy;
        policy
            .capabilities
            .push(NxtlinqCapabilityParams::McpConnect {
                servers: vec!["customer-a".into(), "customer-b".into()],
                approval_required: None,
            });
        policy
            .capabilities
            .push(NxtlinqCapabilityParams::McpInvoke {
                servers: vec!["customer-a".into(), "customer-b".into()],
                tools: vec!["lookup".into()],
                approval_required: None,
            });
        assert!(normalize_policy(policy, project.path())
            .expect_err("one invoke capability must name one server")
            .message
            .contains("exactly one server"));
    }

    #[test]
    fn external_mcp_invocation_requires_a_matching_connection_grant() {
        let project = tempfile::tempdir().expect("tempdir");
        let mut policy = params().policy;
        policy
            .capabilities
            .push(NxtlinqCapabilityParams::McpInvoke {
                servers: vec!["customer-mcp".into()],
                tools: vec!["lookup".into()],
                approval_required: None,
            });
        assert!(normalize_policy(policy, project.path())
            .expect_err("unconnected invocation must fail")
            .message
            .contains("matching mcp:connect"));

        let mut policy = params().policy;
        policy
            .capabilities
            .push(NxtlinqCapabilityParams::McpConnect {
                servers: vec!["customer-mcp".into()],
                approval_required: None,
            });
        policy
            .capabilities
            .push(NxtlinqCapabilityParams::McpInvoke {
                servers: vec!["customer-mcp".into()],
                tools: vec!["lookup".into()],
                approval_required: None,
            });
        normalize_policy(policy, project.path()).expect("connected invocation");
    }

    #[test]
    fn accepts_an_initialized_project() {
        let root = tempfile::tempdir().expect("tempdir");
        let project = root.path().join("project");
        let nxtlinq = project.join("nxtlinq");
        std::fs::create_dir_all(&nxtlinq).expect("nxtlinq dir");
        std::fs::write(nxtlinq.join("agent.manifest.json"), "{}").expect("manifest fixture");
        assert_eq!(
            validate_project_root(project.to_str().expect("UTF-8 path")).expect("valid"),
            project.canonicalize().expect("canonical project")
        );
    }

    #[test]
    fn accepts_an_uninitialized_project_for_desktop_review() {
        let project = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            validate_project_root(project.path().to_str().expect("UTF-8 path")).expect("valid"),
            project.path().canonicalize().expect("canonical project")
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_symlinked_project_root() {
        let root = tempfile::tempdir().expect("tempdir");
        let project = root.path().join("project");
        let link = root.path().join("project-link");
        std::fs::create_dir(&project).expect("project dir");
        std::os::unix::fs::symlink(&project, &link).expect("project symlink");
        let error = validate_project_root(link.to_str().expect("UTF-8 path"))
            .expect_err("must reject symlink");
        assert!(error.message.contains("real directory, not a symlink"));
    }
}
