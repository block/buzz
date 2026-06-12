use std::collections::BTreeMap;

use super::types::{ExtensionEntry, RuntimeFileConfig};

/// Read Codex config from `~/.codex/config.toml` (or `$CODEX_HOME/config.toml`).
pub(super) fn read_config_file() -> Option<RuntimeFileConfig> {
    let path = codex_config_path()?;
    let raw = std::fs::read_to_string(path).ok()?;
    parse_codex_config(&raw)
}

fn parse_codex_config(toml_str: &str) -> Option<RuntimeFileConfig> {
    let table: toml::Table = toml_str.parse().ok()?;

    let model = toml_string(&table, "model");
    let model_provider = toml_string(&table, "model_provider");
    let approval_policy = toml_string(&table, "approval_policy");
    let sandbox_mode = toml_string(&table, "sandbox_mode");
    let reasoning_effort = toml_string(&table, "model_reasoning_effort");
    let context_window = toml_string(&table, "model_context_window");

    // Two-axis mode: approval_policy × sandbox_mode
    let mode = match (approval_policy.as_deref(), sandbox_mode.as_deref()) {
        (Some(ap), Some(sm)) => Some(format!("{ap}/{sm}")),
        (Some(ap), None) => Some(ap.to_string()),
        (None, Some(sm)) => Some(format!("default/{sm}")),
        (None, None) => None,
    };

    let mut extra = BTreeMap::new();
    if let Some(ref ap) = approval_policy {
        extra.insert("approval_policy".to_string(), ap.clone());
    }
    if let Some(ref sm) = sandbox_mode {
        extra.insert("sandbox_mode".to_string(), sm.clone());
    }

    // MCP servers from [mcp_servers.<id>] tables
    let extensions = parse_mcp_servers(&table);

    // Custom model providers from [model_providers.<id>]
    if let Some(providers) = table.get("model_providers").and_then(|v| v.as_table()) {
        for (name, _) in providers {
            extra.insert(format!("model_providers.{name}"), "configured".to_string());
        }
    }

    Some(RuntimeFileConfig {
        model,
        provider: model_provider,
        mode,
        thinking_effort: reasoning_effort,
        max_output_tokens: None,
        context_limit: context_window,
        system_prompt: toml_string(&table, "instructions"),
        extensions,
        extra,
    })
}

fn parse_mcp_servers(table: &toml::Table) -> Vec<ExtensionEntry> {
    let servers = match table.get("mcp_servers").and_then(|v| v.as_table()) {
        Some(s) => s,
        None => return Vec::new(),
    };

    servers
        .iter()
        .map(|(name, _config)| ExtensionEntry {
            name: name.clone(),
            kind: "mcp".to_string(),
            enabled: true,
        })
        .collect()
}

fn toml_string(table: &toml::Table, key: &str) -> Option<String> {
    table
        .get(key)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn codex_config_path() -> Option<std::path::PathBuf> {
    if let Ok(home) = std::env::var("CODEX_HOME") {
        return Some(std::path::PathBuf::from(home).join("config.toml"));
    }
    let home = dirs::home_dir()?;
    Some(home.join(".codex").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_config() {
        let toml = r#"
model = "o3"
model_provider = "openai"
approval_policy = "unless-allow-listed"
sandbox_mode = "permissive"
model_reasoning_effort = "high"
"#;
        let cfg = parse_codex_config(toml).unwrap();
        assert_eq!(cfg.model.as_deref(), Some("o3"));
        assert_eq!(cfg.provider.as_deref(), Some("openai"));
        assert_eq!(cfg.mode.as_deref(), Some("unless-allow-listed/permissive"));
        assert_eq!(cfg.thinking_effort.as_deref(), Some("high"));
    }

    #[test]
    fn parse_mcp_servers() {
        let toml = r#"
model = "gpt-4.1"

[mcp_servers.filesystem]
command = "npx"
args = ["-y", "@anthropic-ai/mcp-filesystem"]

[mcp_servers.github]
command = "gh"
"#;
        let cfg = parse_codex_config(toml).unwrap();
        assert_eq!(cfg.extensions.len(), 2);
    }

    #[test]
    fn parse_custom_providers() {
        let toml = r#"
model = "my-model"
model_provider = "custom-provider"

[model_providers.custom-provider]
base_url = "http://localhost:8080"
"#;
        let cfg = parse_codex_config(toml).unwrap();
        assert_eq!(cfg.provider.as_deref(), Some("custom-provider"));
        assert!(cfg.extra.contains_key("model_providers.custom-provider"));
    }

    #[test]
    fn approval_only_mode() {
        let toml = r#"approval_policy = "on-failure""#;
        let cfg = parse_codex_config(toml).unwrap();
        assert_eq!(cfg.mode.as_deref(), Some("on-failure"));
    }

    #[test]
    fn sandbox_only_mode() {
        let toml = r#"sandbox_mode = "strict""#;
        let cfg = parse_codex_config(toml).unwrap();
        assert_eq!(cfg.mode.as_deref(), Some("default/strict"));
    }

    #[test]
    fn empty_config() {
        let cfg = parse_codex_config("").unwrap();
        assert!(cfg.model.is_none());
        assert!(cfg.provider.is_none());
        assert!(cfg.mode.is_none());
    }

    #[test]
    fn invalid_toml_returns_none() {
        assert!(parse_codex_config("{{{{not valid").is_none());
    }
}
