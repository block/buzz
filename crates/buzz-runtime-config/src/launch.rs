//! Mapper from the unified native model to the versioned stdio MCP launch
//! document consumed by agent launchers.
//!
//! [`RuntimeMcpConfig`] is the runtime-agnostic view read from an external
//! framework's native config (Hermes YAML / Kimi Code JSON). The launcher-side
//! contract — the wire document that #5349 browser_harness consumes — is a
//! separate, stricter, versioned shape. This module translates the former into
//! the latter.
//!
//! The target schema mirrors buzz-core's `mcp_config` v1 launch document
//! (`transport: "stdio"`), and mirrors its validation so a mapping that cannot
//! be represented as a valid launch document is rejected loudly — an enabled
//! native server whose name or environment fails the stricter launch-schema
//! rules surfaces as an error naming the server, never silently dropped.
use std::collections::{BTreeMap, HashSet};

use serde::Serialize;

use crate::model::{McpServerConfig, RuntimeKind, RuntimeMcpConfig};

/// Wire-schema version of the launch document we emit (buzz-core v1).
pub const MCP_LAUNCH_DOC_VERSION: u32 = 1;
/// Transport tag of the emitted launch entries (v1 supports stdio only).
pub const STDIO_TRANSPORT: &str = "stdio";

/// Harness identity/authorization variables the launch schema forbids a server
/// from configuring. Kept in sync with buzz-core's `PROTECTED_MCP_ENV_NAMES`.
pub const PROTECTED_MCP_ENV_NAMES: [&str; 6] = [
    "BUZZ_PRIVATE_KEY",
    "NOSTR_PRIVATE_KEY",
    "BUZZ_AUTH_TAG",
    "BUZZ_API_TOKEN",
    "BUZZ_ACP_PRIVATE_KEY",
    "BUZZ_ACP_API_TOKEN",
];

/// One native server mapped into launch-document form.
///
/// Field order and `serde` tagging match the versioned stdio MCP launch
/// document so the emitted JSON is directly consumable by a parse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LaunchStdioServer {
    /// `transport` tag from the launch schema (always `"stdio"`).
    #[serde(rename = "transport")]
    transport: &'static str,
    /// Stable server name.
    name: String,
    /// Executable invoked directly without shell parsing.
    command: String,
    /// Arguments passed to `command` in configured order.
    args: Vec<String>,
    /// Server-specific environment in deterministic key order.
    env: BTreeMap<String, String>,
}

/// A versioned collection of stdio MCP servers ready for agent launch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpLaunchConfigDocument {
    /// Wire-schema version.
    version: u32,
    /// Ordered stdio servers in launch order.
    servers: Vec<LaunchStdioServer>,
}

impl McpLaunchConfigDocument {
    /// Return the mapped launch entries in launch order.
    pub fn servers(&self) -> &[LaunchStdioServer] {
        &self.servers
    }

    /// Map every *enabled* native server into a versioned stdio launch
    /// document.
    ///
    /// Disabled servers (`enabled: Some(false)`) are excluded. Servers whose
    /// native entry does not specify `enabled` (treat as enabled) are included.
    /// Field values map 1:1 from the native entry into the launch entry.
    pub fn from_runtime_config(config: &RuntimeMcpConfig) -> Self {
        let servers = config
            .servers
            .iter()
            .filter(|server| server.is_enabled())
            .map(native_to_launch)
            .collect();
        Self {
            version: MCP_LAUNCH_DOC_VERSION,
            servers,
        }
    }

    /// Encode this document as JSON after validating it against the launch
    /// schema (mirrors `buzz-core::mcp_config::McpLaunchConfigDocument`).
    pub fn to_json(&self) -> Result<Vec<u8>, LaunchMapError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|err| LaunchMapError::Encode(err.to_string()))
    }

    /// Validate the mapped document against the launch-schema rules. Returns
    /// an error naming the first offending server; a document that validates
    /// here is guaranteed to be accepted by the launch-schema parser.
    pub fn validate(&self) -> Result<(), LaunchMapError> {
        if self.version != MCP_LAUNCH_DOC_VERSION {
            return Err(LaunchMapError::Invalid(format!(
                "unsupported MCP config version {} (expected {MCP_LAUNCH_DOC_VERSION})",
                self.version
            )));
        }
        let mut names = HashSet::with_capacity(self.servers.len());
        for server in &self.servers {
            if !valid_server_name(&server.name) {
                return Err(LaunchMapError::Invalid(format!(
                    "MCP server '{}' has invalid name: use 1..=128 ASCII letters, digits, underscores, or hyphens, without '__'",
                    server.name
                )));
            }
            if !names.insert(server.name.as_str()) {
                return Err(LaunchMapError::Invalid(format!(
                    "duplicate MCP server name '{}'",
                    server.name
                )));
            }
            if server.command.trim().is_empty() || server.command.contains('\0') {
                return Err(LaunchMapError::Invalid(format!(
                    "MCP server '{}' command must not be blank and must contain no NUL bytes",
                    server.name
                )));
            }
            if server.args.iter().any(|argument| argument.contains('\0')) {
                return Err(LaunchMapError::Invalid(format!(
                    "MCP server '{}' arguments must contain no NUL bytes",
                    server.name
                )));
            }
            let mut env_names = HashSet::with_capacity(server.env.len());
            for (key, value) in &server.env {
                if !valid_env_name(key) {
                    return Err(LaunchMapError::Invalid(format!(
                        "MCP server '{}' environment key '{key}' is invalid (must start with an ASCII letter or '_' and contain only letters, digits, '_')",
                        server.name
                    )));
                }
                if !env_names.insert(key.to_ascii_uppercase()) {
                    return Err(LaunchMapError::Invalid(format!(
                        "MCP server '{}' has environment keys that differ only by ASCII case",
                        server.name
                    )));
                }
                if PROTECTED_MCP_ENV_NAMES
                    .iter()
                    .any(|protected| key.eq_ignore_ascii_case(protected))
                {
                    return Err(LaunchMapError::Invalid(format!(
                        "MCP server '{}' may not configure a protected environment key",
                        server.name
                    )));
                }
                if value.contains('\0') {
                    return Err(LaunchMapError::Invalid(format!(
                        "MCP server '{}' environment entry '{key}' contains a NUL byte",
                        server.name
                    )));
                }
            }
        }
        Ok(())
    }
}

/// Map one native server into launch-document form. Field values are copied
/// verbatim — this mapper performs a structural translation, not a value
/// rewrite.
fn native_to_launch(server: &McpServerConfig) -> LaunchStdioServer {
    LaunchStdioServer {
        transport: STDIO_TRANSPORT,
        name: server.name.clone(),
        command: server.command.clone(),
        args: server.args.clone(),
        env: server.env.clone(),
    }
}

/// Mirror of the launch-schema's `valid_mcp_server_name`.
fn valid_server_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.contains("__")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

/// Mirror of the launch-schema's `valid_mcp_env_name`.
fn valid_env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(byte) if byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// A mapping-time failure: either the document cannot be encoded or a mapped
/// server would not be accepted by the launch schema.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LaunchMapError {
    /// The document violates the launch schema.
    #[error("{0}")]
    Invalid(String),
    /// The document is valid but failed to serialize.
    #[error("failed to encode MCP launch document: {0}")]
    Encode(String),
}

/// Convenience: map a native runtime config straight to encoded launch JSON.
///
/// Returns the encoded document bytes on success. On failure, names the
/// offending server.
pub fn to_launch_json(config: &RuntimeMcpConfig) -> Result<Vec<u8>, LaunchMapError> {
    let document = McpLaunchConfigDocument::from_runtime_config(config);
    document.to_json()
}

/// Human-readable runtime label for diagnostics.
pub fn runtime_label(runtime: RuntimeKind) -> &'static str {
    match runtime {
        RuntimeKind::Hermes => "Hermes",
        RuntimeKind::KimiCode => "Kimi Code",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn native(name: &str, command: &str, enabled: Option<bool>) -> McpServerConfig {
        McpServerConfig {
            name: name.to_string(),
            command: command.to_string(),
            args: vec![],
            env: BTreeMap::new(),
            enabled,
        }
    }

    fn config(servers: Vec<McpServerConfig>) -> RuntimeMcpConfig {
        RuntimeMcpConfig {
            runtime: RuntimeKind::Hermes,
            servers,
        }
    }

    #[test]
    fn maps_enabled_servers_and_skips_disabled() {
        let cfg = config(vec![
            native("on", "npx", Some(true)),
            native("off", "npx", Some(false)),
            native("unset", "uv", None),
        ]);
        let doc = McpLaunchConfigDocument::from_runtime_config(&cfg);
        let names: Vec<_> = doc.servers.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["on", "unset"]);
        assert_eq!(doc.version, MCP_LAUNCH_DOC_VERSION);
    }

    #[test]
    fn wire_carries_stdio_transport_tag() {
        let cfg = config(vec![native("sciverse", "npx", None)]);
        let json =
            serde_json::to_value(McpLaunchConfigDocument::from_runtime_config(&cfg)).unwrap();
        assert_eq!(json["version"], 1);
        assert_eq!(json["servers"][0]["transport"], "stdio");
        assert_eq!(json["servers"][0]["name"], "sciverse");
        assert_eq!(json["servers"][0]["command"], "npx");
    }

    #[test]
    fn maps_env_verbatim() {
        let mut server = native("sciverse", "npx", None);
        server
            .args
            .extend(["-y".to_string(), "sciverse-mcp-server".to_string()]);
        server
            .env
            .insert("SCIVERSE_API_TOKEN".to_string(), "token-value".to_string());
        let cfg = config(vec![server]);
        let json =
            serde_json::to_value(McpLaunchConfigDocument::from_runtime_config(&cfg)).unwrap();
        assert_eq!(json["servers"][0]["args"][0], "-y");
        assert_eq!(
            json["servers"][0]["env"]["SCIVERSE_API_TOKEN"],
            "token-value"
        );
    }

    #[test]
    fn rejects_duplicate_names() {
        let cfg = config(vec![native("dup", "npx", None), native("dup", "uv", None)]);
        let doc = McpLaunchConfigDocument::from_runtime_config(&cfg);
        let err = doc.validate().unwrap_err();
        assert!(err.to_string().contains("duplicate MCP server name"));
    }

    #[test]
    fn rejects_protected_env_key() {
        let mut server = native("one", "npx", None);
        server
            .env
            .insert("BUZZ_PRIVATE_KEY".to_string(), "secret".to_string());
        let cfg = config(vec![server]);
        let doc = McpLaunchConfigDocument::from_runtime_config(&cfg);
        let err = doc.validate().unwrap_err();
        assert!(err.to_string().contains("protected environment key"));
    }

    #[test]
    fn rejects_env_keys_differing_only_by_case() {
        let mut server = native("one", "npx", None);
        server.env.insert("Token".to_string(), "a".to_string());
        server.env.insert("TOKEN".to_string(), "b".to_string());
        let cfg = config(vec![server]);
        let doc = McpLaunchConfigDocument::from_runtime_config(&cfg);
        let err = doc.validate().unwrap_err();
        assert!(err.to_string().contains("differ only by ASCII case"));
    }

    #[test]
    fn roundtrip_real_sciverse_entry_validates() {
        // The real Hermes `sciverse` entry (redacted token) must map cleanly.
        let mut server = native("sciverse", "npx", Some(true));
        server
            .args
            .extend(["-y".to_string(), "sciverse-mcp-server".to_string()]);
        server
            .env
            .insert("SCIVERSE_API_TOKEN".to_string(), "sci_redacted".to_string());
        let cfg = config(vec![server]);
        let bytes = to_launch_json(&cfg).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed["servers"][0]["name"], "sciverse");
        assert_eq!(
            parsed["servers"][0]["env"]["SCIVERSE_API_TOKEN"],
            "sci_redacted"
        );
    }
}
