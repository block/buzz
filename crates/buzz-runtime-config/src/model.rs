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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Server name (the map key in the runtime's native config).
    pub name: String,

    /// Executable used to launch the server (e.g. `npx`, `uv`).
    pub command: String,

    /// Arguments passed to `command`.
    #[serde(default)]
    pub args: Vec<String>,

    /// Environment variables passed to the server process. May contain
    /// secrets — never log or persist values directly; use [`Self::redacted`]
    /// for display.
    #[serde(default)]
    pub env: BTreeMap<String, String>,

    /// Whether the server is enabled. `None` means the native config did
    /// not specify it (treat as enabled).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

impl McpServerConfig {
    /// Whether this server is active. Servers whose native config does not
    /// specify `enabled` (`None`) are active by default.
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    /// A copy with every `env` value replaced by a mask, safe to display or
    /// log. Keys are preserved so operators can still see which variables a
    /// server expects.
    pub fn redacted(&self) -> Self {
        Self {
            env: self
                .env
                .keys()
                .map(|k| (k.clone(), "<redacted>".to_string()))
                .collect(),
            ..self.clone()
        }
    }
}

/// The MCP configuration of one runtime, in unified form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeMcpConfig {
    /// Which runtime this configuration was read from.
    pub runtime: RuntimeKind,

    /// MCP servers, ordered by name for stable comparisons and output.
    pub servers: Vec<McpServerConfig>,
}

impl RuntimeMcpConfig {
    /// Servers that are currently active (see [`McpServerConfig::is_enabled`]).
    pub fn enabled_servers(&self) -> impl Iterator<Item = &McpServerConfig> {
        self.servers.iter().filter(|s| s.is_enabled())
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
    fn redacted_masks_env_values_but_keeps_keys() {
        let mut s = server("a", "npx", None);
        s.env
            .insert("API_TOKEN".to_string(), "secret-value".to_string());
        let r = s.redacted();
        assert_eq!(
            r.env.get("API_TOKEN").map(String::as_str),
            Some("<redacted>")
        );
        // Original is untouched.
        assert_eq!(
            s.env.get("API_TOKEN").map(String::as_str),
            Some("secret-value")
        );
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
