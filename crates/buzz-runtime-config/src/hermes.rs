//! Read-only adapter for Hermes MCP configuration.
//!
//! Hermes keeps its configuration in `~/.hermes/config.yaml`. MCP servers
//! live under the top-level `mcp_servers` map:
//!
//! ```yaml
//! mcp_servers:
//!   sciverse:
//!     command: npx
//!     args: ["-y", "@sciverse/mcp"]
//!     env:
//!       SCIVERSE_API_TOKEN: "..."
//!     enabled: true
//! ```
//!
//! Only `mcp_servers` is read; every other top-level key in the file is
//! ignored, so unrelated Hermes settings never fail parsing here.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::ConfigError;
use crate::model::{McpServerConfig, RuntimeKind, RuntimeMcpConfig};

/// Default location of the Hermes config file (`~/.hermes/config.yaml`).
/// Returns `None` when the home directory cannot be determined.
pub fn default_config_path() -> Option<PathBuf> {
    std::env::home_dir().map(|h| h.join(".hermes").join("config.yaml"))
}

/// Read and validate MCP servers from a Hermes `config.yaml`.
pub fn read_mcp_config(path: &Path) -> Result<RuntimeMcpConfig, ConfigError> {
    let content = crate::read_config(path)?;
    parse_mcp_config(&content)
}

/// Parse and validate MCP servers from Hermes `config.yaml` content.
pub fn parse_mcp_config(content: &str) -> Result<RuntimeMcpConfig, ConfigError> {
    crate::validate_config_size(content.len())?;
    let raw: RawHermesConfig = serde_yaml::from_str(content)?;
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
        runtime: RuntimeKind::Hermes,
        servers,
    }
    .validated()
}

/// Intentionally permissive view of `config.yaml`: only `mcp_servers` is
/// modeled. The real file carries many unrelated sections (teams, model
/// aliases, ...); unknown keys are silently ignored.
#[derive(Debug, Deserialize)]
struct RawHermesConfig {
    #[serde(default)]
    mcp_servers: BTreeMap<String, RawHermesServer>,
}

#[derive(Debug, Deserialize)]
struct RawHermesServer {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    enabled: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_config_ignores_unrelated_sections() {
        let yaml = r#"
teams:
  google_chat: {}
mcp_servers:
  sciverse:
    command: npx
    args:
      - "-y"
      - "@sciverse/mcp"
    env:
      SCIVERSE_API_TOKEN: "token-value"
    enabled: true
  local-tool:
    command: uv
    args: ["run", "tool"]
    enabled: false
model_aliases:
  kimi-k3:
    model: kimi-k3
"#;
        let cfg = parse_mcp_config(yaml).unwrap();
        assert_eq!(cfg.runtime, RuntimeKind::Hermes);
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
        assert_eq!(sci.args, vec!["-y", "@sciverse/mcp"]);
        assert_eq!(
            sci.env.get("SCIVERSE_API_TOKEN").map(String::as_str),
            Some("token-value")
        );
        assert_eq!(sci.enabled, Some(true));
    }

    #[test]
    fn missing_mcp_servers_yields_empty_config() {
        let cfg = parse_mcp_config("teams: {}\n").unwrap();
        assert!(cfg.servers.is_empty());
    }

    #[test]
    fn missing_optional_fields_default() {
        let yaml = r#"
mcp_servers:
  minimal:
    command: npx
"#;
        let cfg = parse_mcp_config(yaml).unwrap();
        let s = &cfg.servers[0];
        assert!(s.args.is_empty());
        assert!(s.env.is_empty());
        assert_eq!(s.enabled, None);
        assert!(s.is_enabled());
    }

    #[test]
    fn empty_command_is_a_validation_error() {
        let yaml = r#"
mcp_servers:
  broken:
    command: ""
"#;
        let err = parse_mcp_config(yaml).unwrap_err();
        assert!(matches!(err, ConfigError::Validation(_)));
        assert!(err.to_string().contains("broken"));
    }

    #[test]
    fn malformed_yaml_errors() {
        let err = parse_mcp_config("mcp_servers: [not, a, map").unwrap_err();
        assert!(matches!(err, ConfigError::Yaml(_)));
    }

    #[test]
    fn read_missing_file_errors() {
        let err = read_mcp_config(Path::new("/nonexistent/config.yaml")).unwrap_err();
        assert!(matches!(err, ConfigError::Io(_)));
    }

    #[test]
    fn read_from_disk_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(
            &path,
            "mcp_servers:\n  t:\n    command: npx\n    enabled: true\n",
        )
        .unwrap();
        let cfg = read_mcp_config(&path).unwrap();
        assert_eq!(cfg.servers.len(), 1);
        assert_eq!(cfg.servers[0].name, "t");
    }

    #[test]
    fn oversized_config_is_rejected_before_parsing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
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
