use super::types::{ExtensionEntry, RuntimeFileConfig};

/// Read Claude Code config from `~/.claude/settings.json` and `~/.claude.json`.
pub(super) fn read_config_file() -> Option<RuntimeFileConfig> {
    let home = dirs::home_dir()?;
    let settings_path = home.join(".claude").join("settings.json");
    let mcp_path = home.join(".claude.json");

    let settings = read_json_file(&settings_path);
    let mcp_config = read_json_file(&mcp_path);

    if settings.is_none() && mcp_config.is_none() {
        return None;
    }

    let mut cfg = RuntimeFileConfig::default();

    if let Some(ref s) = settings {
        cfg.model = json_string(s, "model");

        if let Some(permissions) = s.get("permissions") {
            if let Some(mode) = permissions.get("default").and_then(|v| v.as_str()) {
                cfg.extra
                    .insert("permissions.default".to_string(), mode.to_string());
            }
        }

        if s.get("hooks").is_some() {
            cfg.extra
                .insert("hooks".to_string(), "configured".to_string());
        }

        if let Some(style) = json_string(s, "outputStyle") {
            cfg.extra.insert("outputStyle".to_string(), style);
        }
    }

    // MCP servers from ~/.claude.json
    let mut extensions = Vec::new();
    if let Some(ref mc) = mcp_config {
        if let Some(servers) = mc.get("mcpServers").and_then(|v| v.as_object()) {
            for (name, _config) in servers {
                extensions.push(ExtensionEntry {
                    name: name.clone(),
                    kind: "mcp".to_string(),
                    enabled: true,
                });
            }
        }
    }
    cfg.extensions = extensions;

    // Provider is always Anthropic for Claude Code.
    cfg.extra
        .insert("provider_locked".to_string(), "true".to_string());

    Some(cfg)
}

fn read_json_file(path: &std::path::Path) -> Option<serde_json::Value> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn json_string(val: &serde_json::Value, key: &str) -> Option<String> {
    val.get(key)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_settings(json: &str) -> RuntimeFileConfig {
        use std::collections::BTreeMap;
        let val: serde_json::Value = serde_json::from_str(json).unwrap();
        let mut extra = BTreeMap::new();
        if let Some(permissions) = val.get("permissions") {
            if let Some(mode) = permissions.get("default").and_then(|v| v.as_str()) {
                extra.insert("permissions.default".to_string(), mode.to_string());
            }
        }
        if val.get("hooks").is_some() {
            extra.insert("hooks".to_string(), "configured".to_string());
        }
        if let Some(style) = json_string(&val, "outputStyle") {
            extra.insert("outputStyle".to_string(), style);
        }
        RuntimeFileConfig {
            model: json_string(&val, "model"),
            system_prompt: None,
            extra,
            ..Default::default()
        }
    }

    #[test]
    fn parse_model_from_settings() {
        let cfg = parse_settings(r#"{"model": "claude-sonnet-4-20250514"}"#);
        assert_eq!(cfg.model.as_deref(), Some("claude-sonnet-4-20250514"));
    }

    #[test]
    fn parse_permissions_and_hooks() {
        let cfg = parse_settings(
            r#"{"permissions": {"default": "bypassPermissions"}, "hooks": {"pre-commit": {}}}"#,
        );
        assert_eq!(
            cfg.extra.get("permissions.default").map(|s| s.as_str()),
            Some("bypassPermissions")
        );
        assert_eq!(
            cfg.extra.get("hooks").map(|s| s.as_str()),
            Some("configured")
        );
    }

    #[test]
    fn parse_output_style_in_extra() {
        let cfg = parse_settings(r#"{"outputStyle": "Be concise and technical"}"#);
        assert_eq!(
            cfg.extra.get("outputStyle").map(|s| s.as_str()),
            Some("Be concise and technical")
        );
        assert!(cfg.system_prompt.is_none());
    }

    #[test]
    fn parse_mcp_servers() {
        let json =
            r#"{"mcpServers": {"filesystem": {"command": "npx"}, "github": {"command": "gh"}}}"#;
        let val: serde_json::Value = serde_json::from_str(json).unwrap();
        let mut extensions = Vec::new();
        if let Some(servers) = val.get("mcpServers").and_then(|v| v.as_object()) {
            for (name, _) in servers {
                extensions.push(ExtensionEntry {
                    name: name.clone(),
                    kind: "mcp".to_string(),
                    enabled: true,
                });
            }
        }
        assert_eq!(extensions.len(), 2);
    }

    #[test]
    fn empty_settings_returns_defaults() {
        let cfg = parse_settings("{}");
        assert!(cfg.model.is_none());
        assert!(cfg.system_prompt.is_none());
    }
}
