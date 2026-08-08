//! Read-only adapter for Kimi Code MCP configuration.
//!
//! Kimi Code keeps its MCP configuration in `~/.kimi-code/mcp.json`
//! (or `$KIMI_CODE_HOME/mcp.json`). MCP servers live under the top-level
//! `mcpServers` map:
//!
//! ```json
//! {
//!   "mcpServers": {
//!     "sciverse": {
//!       "command": "npx",
//!       "args": ["-y", "sciverse-mcp-server"],
//!       "env": { "SCIVERSE_API_TOKEN": "..." }
//!     }
//!   }
//! }
//! ```
//!
//! Only `mcpServers` is read; every other top-level key in the file is
//! ignored, so unrelated settings never fail parsing here.
//!
//! Limitations (PR-0 scope): the unified model currently covers stdio
//! servers only. Kimi Code also supports HTTP/SSE servers declared with a
//! `url` field instead of `command` (plus `headers`, `bearerTokenEnvVar`,
//! `transport: "sse"`); such entries surface here as a validation error
//! naming the server rather than being silently dropped. Timeouts and tool
//! allow/block lists (`startupTimeoutMs`, `toolTimeoutMs`, `enabledTools`,
//! `disabledTools`) are likewise not modeled yet.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::ConfigError;
use crate::model::{McpServerConfig, RuntimeKind, RuntimeMcpConfig};

/// Default location of the Kimi Code MCP config file
/// (`$KIMI_CODE_HOME/mcp.json`, else `~/.kimi-code/mcp.json`).
/// Returns `None` when neither is set and the home directory cannot be
/// determined.
pub fn default_config_path() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("KIMI_CODE_HOME") {
        return Some(PathBuf::from(home).join("mcp.json"));
    }
    std::env::home_dir().map(|h| h.join(".kimi-code").join("mcp.json"))
}

/// Read and validate MCP servers from a Kimi Code `mcp.json`.
pub fn read_mcp_config(path: &Path) -> Result<RuntimeMcpConfig, ConfigError> {
    let content = crate::read_config(path)?;
    parse_mcp_config(&content)
}

/// Parse and validate MCP servers from Kimi Code `mcp.json` content.
pub fn parse_mcp_config(content: &str) -> Result<RuntimeMcpConfig, ConfigError> {
    crate::validate_config_size(content.len())?;
    let raw: RawKimiConfig = serde_json::from_str(content)?;
    let mut servers: Vec<McpServerConfig> = raw
        .mcp_servers
        .into_iter()
        .map(|(name, srv)| McpServerConfig {
            name,
            command: srv.command,
            args: srv.args,
            env: srv.env,
            enabled: srv.enabled,
        })
        .collect();
    servers.sort_by(|a, b| a.name.cmp(&b.name));
    RuntimeMcpConfig {
        runtime: RuntimeKind::KimiCode,
        servers,
    }
    .validated()
}

/// Intentionally permissive view of `mcp.json`: only `mcpServers` is
/// modeled. Unknown keys are silently ignored.
#[derive(Debug, Deserialize)]
struct RawKimiConfig {
    #[serde(default, rename = "mcpServers")]
    mcp_servers: BTreeMap<String, RawKimiServer>,
}

#[derive(Debug, Deserialize)]
struct RawKimiServer {
    /// Empty for HTTP/SSE (`url`-based) servers, which the unified model
    /// does not represent yet; validation rejects them loudly.
    #[serde(default)]
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    /// Kimi Code supports `enabled` (default: enabled). Absent maps to
    /// `None`, which [`McpServerConfig::is_enabled`] treats as enabled.
    #[serde(default)]
    enabled: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_config_ignores_unrelated_keys() {
        let json = r#"{
            "someOtherSetting": true,
            "mcpServers": {
                "sciverse": {
                    "command": "npx",
                    "args": ["-y", "sciverse-mcp-server"],
                    "env": {"SCIVERSE_API_TOKEN": "token-value"}
                },
                "local-tool": {
                    "command": "uv",
                    "args": ["run", "tool"],
                    "enabled": false
                }
            }
        }"#;
        let cfg = parse_mcp_config(json).unwrap();
        assert_eq!(cfg.runtime, RuntimeKind::KimiCode);
        assert_eq!(cfg.servers.len(), 2);

        let local = &cfg.servers[0];
        assert_eq!(local.name, "local-tool");
        assert_eq!(local.command, "uv");
        assert_eq!(local.args, vec!["run", "tool"]);
        assert_eq!(local.enabled, Some(false));
        assert!(!local.is_enabled());

        let sci = &cfg.servers[1];
        assert_eq!(sci.name, "sciverse");
        assert_eq!(sci.command, "npx");
        assert_eq!(sci.args, vec!["-y", "sciverse-mcp-server"]);
        assert_eq!(
            sci.env.get("SCIVERSE_API_TOKEN").map(String::as_str),
            Some("token-value")
        );
        assert_eq!(sci.enabled, None);
        assert!(sci.is_enabled());
    }

    #[test]
    fn missing_mcp_servers_yields_empty_config() {
        let cfg = parse_mcp_config("{}").unwrap();
        assert!(cfg.servers.is_empty());
    }

    #[test]
    fn missing_optional_fields_default() {
        let json = r#"{"mcpServers": {"minimal": {"command": "npx"}}}"#;
        let cfg = parse_mcp_config(json).unwrap();
        let s = &cfg.servers[0];
        assert!(s.args.is_empty());
        assert!(s.env.is_empty());
        assert_eq!(s.enabled, None);
        assert!(s.is_enabled());
    }

    #[test]
    fn empty_command_is_a_validation_error() {
        // Also how `url`-based (HTTP/SSE) servers surface until the unified
        // model grows a transport field: loud error, never silently dropped.
        let json = r#"{"mcpServers": {"broken": {"command": ""}}}"#;
        let err = parse_mcp_config(json).unwrap_err();
        assert!(matches!(err, ConfigError::Validation(_)));
        assert!(err.to_string().contains("broken"));
    }

    #[test]
    fn url_based_server_is_rejected_loudly() {
        let json = r#"{"mcpServers": {"remote": {"url": "https://mcp.example.com/mcp"}}}"#;
        let err = parse_mcp_config(json).unwrap_err();
        assert!(matches!(err, ConfigError::Validation(_)));
        assert!(err.to_string().contains("remote"));
    }

    #[test]
    fn malformed_json_errors() {
        let err = parse_mcp_config("{\"mcpServers\": [not, a, map").unwrap_err();
        assert!(matches!(err, ConfigError::Json(_)));
    }

    #[test]
    fn read_missing_file_errors() {
        let err = read_mcp_config(Path::new("/nonexistent/mcp.json")).unwrap_err();
        assert!(matches!(err, ConfigError::Io(_)));
    }

    #[test]
    fn read_from_disk_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        std::fs::write(
            &path,
            r#"{"mcpServers": {"t": {"command": "npx", "enabled": true}}}"#,
        )
        .unwrap();
        let cfg = read_mcp_config(&path).unwrap();
        assert_eq!(cfg.servers.len(), 1);
        assert_eq!(cfg.servers[0].name, "t");
        assert_eq!(cfg.servers[0].enabled, Some(true));
    }

    #[test]
    fn oversized_config_is_rejected_before_parsing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        std::fs::write(
            &path,
            vec![b'x'; crate::RUNTIME_CONFIG_MAX_BYTES as usize + 1],
        )
        .unwrap();

        let err = read_mcp_config(&path).unwrap_err();
        assert!(matches!(err, ConfigError::TooLarge { .. }));
    }

    #[test]
    fn oversized_in_memory_config_is_rejected_before_parsing() {
        let content = "x".repeat(crate::RUNTIME_CONFIG_MAX_BYTES as usize + 1);
        let err = parse_mcp_config(&content).unwrap_err();
        assert!(matches!(err, ConfigError::TooLarge { .. }));
    }
}
