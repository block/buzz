//! Unified, runtime-agnostic MCP configuration model.
//!
//! The model captures the common denominator of external agent runtimes'
//! MCP server configuration. Runtime-specific concepts are represented
//! explicitly as optional fields:
//!
//! - `enabled`: both Hermes and Kimi Code support a per-server `enabled`
//!   flag (Kimi Code defaults to enabled when the key is absent). Adapters
//!   leave it `None` when the native config does not specify it, which
//!   callers must treat as enabled.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::ConfigError;

/// An external agent framework whose MCP configuration Buzz can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    /// Hermes — `~/.hermes/config.yaml`, `mcp_servers` map (YAML).
    Hermes,
    /// Kimi Code — `~/.kimi-code/mcp.json`, `mcpServers` map (JSON).
    KimiCode,
}

/// A single MCP server entry in the unified model.
#[derive(Clone, PartialEq, Eq)]
pub struct McpServerConfig {
    /// Server name (the map key in the runtime's native config).
    pub name: String,

    /// Executable used to launch the server (e.g. `npx`, `uv`).
    pub command: String,

    /// Arguments passed to `command`.
    pub args: Vec<String>,

    /// Environment variables passed to the server process. May contain
    /// secrets — never log or persist values directly; use [`Self::redacted`]
    /// for display.
    pub env: BTreeMap<String, String>,

    /// Whether the server is enabled. `None` means the native config did
    /// not specify it (treat as enabled).
    pub enabled: Option<bool>,
}

impl std::fmt::Debug for McpServerConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpServerConfig")
            .field("name", &self.name)
            .field("command", &self.command)
            .field("arg_count", &self.args.len())
            .field("env_names", &self.env.keys().collect::<Vec<_>>())
            .field("enabled", &self.enabled)
            .finish()
    }
}

/// Secret-free view of one native MCP server for inventory and display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpServerInventoryEntry {
    /// Server name from the native runtime configuration.
    pub name: String,
    /// Executable configured for the server.
    pub command: String,
    /// Number of configured arguments. Argument values may contain secrets and are omitted.
    pub arg_count: usize,
    /// Environment variable names expected by the server. Values are omitted.
    pub env_names: Vec<String>,
    /// Whether the native runtime enables this server.
    pub enabled: bool,
}

impl McpServerConfig {
    /// Whether this server is active. Servers whose native config does not
    /// specify `enabled` (`None`) are active by default.
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    /// Return a secret-free inventory view safe to display or serialize.
    pub fn redacted(&self) -> McpServerInventoryEntry {
        McpServerInventoryEntry {
            name: self.name.clone(),
            command: self.command.clone(),
            arg_count: self.args.len(),
            env_names: self.env.keys().cloned().collect(),
            enabled: self.is_enabled(),
        }
    }
}

/// The MCP configuration of one runtime, in unified form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeMcpConfig {
    /// Which runtime this configuration was read from.
    pub runtime: RuntimeKind,

    /// MCP servers, ordered by name for stable comparisons and output.
    pub servers: Vec<McpServerConfig>,
}

/// Secret-free native MCP inventory for one runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeMcpInventory {
    /// Runtime that supplied this inventory.
    pub runtime: RuntimeKind,
    /// Native MCP servers with arguments and environment values omitted.
    pub servers: Vec<McpServerInventoryEntry>,
}

impl RuntimeMcpConfig {
    /// Servers that are currently active (see [`McpServerConfig::is_enabled`]).
    pub fn enabled_servers(&self) -> impl Iterator<Item = &McpServerConfig> {
        self.servers.iter().filter(|s| s.is_enabled())
    }

    /// Return a secret-free inventory view safe to display or serialize.
    pub fn redacted(&self) -> RuntimeMcpInventory {
        RuntimeMcpInventory {
            runtime: self.runtime,
            servers: self.servers.iter().map(McpServerConfig::redacted).collect(),
        }
    }

    /// Validate the configuration, returning all issues found.
    ///
    /// Validation is structural only: it checks that entries are well-formed
    /// (non-empty name and command), not that the referenced executables
    /// exist.
    pub fn validate(&self) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for server in &self.servers {
            if server.name.trim().is_empty() {
                issues.push(ValidationIssue::EmptyServerName);
            } else if !seen.insert(server.name.clone()) {
                issues.push(ValidationIssue::DuplicateServerName(server.name.clone()));
            }
            if server.command.trim().is_empty() {
                issues.push(ValidationIssue::EmptyCommand(server.name.clone()));
            }
        }
        issues
    }

    /// Validate, returning `Err` summarizing the issues if any exist.
    pub fn validated(self) -> Result<Self, ConfigError> {
        let issues = self.validate();
        if issues.is_empty() {
            return Ok(self);
        }
        let summary = issues
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ");
        Err(ConfigError::Validation(summary))
    }
}

/// A single structural problem found by [`RuntimeMcpConfig::validate`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValidationIssue {
    #[error("server entry with an empty name")]
    EmptyServerName,

    #[error("duplicate server name: {0}")]
    DuplicateServerName(String),

    #[error("server {0:?} has an empty command")]
    EmptyCommand(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(name: &str, command: &str, enabled: Option<bool>) -> McpServerConfig {
        McpServerConfig {
            name: name.to_string(),
            command: command.to_string(),
            args: vec![],
            env: BTreeMap::new(),
            enabled,
        }
    }

    #[test]
    fn none_enabled_means_active() {
        assert!(server("a", "npx", None).is_enabled());
        assert!(server("a", "npx", Some(true)).is_enabled());
        assert!(!server("a", "npx", Some(false)).is_enabled());
    }

    #[test]
    fn redacted_inventory_omits_argument_and_env_values() {
        let mut s = server("a", "npx", None);
        s.args.push("argument-secret".to_string());
        s.env
            .insert("API_TOKEN".to_string(), "secret-value".to_string());
        let r = s.redacted();
        assert_eq!(r.arg_count, 1);
        assert_eq!(r.env_names, vec!["API_TOKEN"]);
        let serialized = serde_json::to_string(&r).unwrap();
        assert!(!serialized.contains("argument-secret"));
        assert!(!serialized.contains("secret-value"));
        assert!(serialized.contains("API_TOKEN"));
    }

    #[test]
    fn debug_output_omits_argument_and_env_values() {
        let mut s = server("a", "npx", None);
        s.args.push("argument-secret".to_string());
        s.env
            .insert("API_TOKEN".to_string(), "secret-value".to_string());

        let debug = format!("{s:?}");
        assert!(!debug.contains("argument-secret"));
        assert!(!debug.contains("secret-value"));
        assert!(debug.contains("API_TOKEN"));
    }

    #[test]
    fn enabled_servers_filters_disabled() {
        let cfg = RuntimeMcpConfig {
            runtime: RuntimeKind::Hermes,
            servers: vec![
                server("on", "npx", Some(true)),
                server("off", "npx", Some(false)),
                server("kimi-style", "uv", None),
            ],
        };
        let names: Vec<&str> = cfg.enabled_servers().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["on", "kimi-style"]);
    }

    #[test]
    fn validate_clean_config_has_no_issues() {
        let cfg = RuntimeMcpConfig {
            runtime: RuntimeKind::KimiCode,
            servers: vec![server("a", "npx", None), server("b", "uv", None)],
        };
        assert!(cfg.validate().is_empty());
    }

    #[test]
    fn validate_catches_empty_command_and_duplicates() {
        let cfg = RuntimeMcpConfig {
            runtime: RuntimeKind::Hermes,
            servers: vec![
                server("a", "", None),
                server("a", "npx", None),
                server("b", "uv", None),
            ],
        };
        let issues = cfg.validate();
        assert!(issues.contains(&ValidationIssue::EmptyCommand("a".into())));
        assert!(issues.contains(&ValidationIssue::DuplicateServerName("a".into())));
        assert_eq!(issues.len(), 2);
    }

    #[test]
    fn validated_err_summarizes_issues() {
        let cfg = RuntimeMcpConfig {
            runtime: RuntimeKind::Hermes,
            servers: vec![server("a", "", None)],
        };
        let err = cfg.validated().unwrap_err();
        assert!(matches!(err, ConfigError::Validation(_)));
        assert!(err.to_string().contains("empty command"));
    }

    #[test]
    fn runtime_kind_serializes_snake_case() {
        let json = serde_json::to_string(&RuntimeKind::KimiCode).unwrap();
        assert_eq!(json, "\"kimi_code\"");
    }
}
