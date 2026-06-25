use super::types::{ExtensionEntry, RuntimeFileConfig};

const CLAUDE_SCHEMA: &str = include_str!("schemas/claude-code-settings.schema.json");

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

        // effortLevel → thinking_effort (direct mapping per spec)
        cfg.thinking_effort = json_string(s, "effortLevel");

        // Schema-driven extra fields — skip normalized keys.
        let skip = &["model", "effortLevel"];
        cfg.extra = super::schema_walker::extract_schema_fields(CLAUDE_SCHEMA, s, skip);

        cfg.schema_version = super::schema_walker::schema_version("claude");
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
    // Buzz-synthesized annotation — not a field from the user's config file.
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

    /// Parse a settings JSON string into a RuntimeFileConfig using the same
    /// logic as read_config_file but without touching the filesystem.
    fn parse_settings(json: &str) -> RuntimeFileConfig {
        let val: serde_json::Value = serde_json::from_str(json).unwrap();
        let mut cfg = RuntimeFileConfig::default();
        cfg.model = json_string(&val, "model");
        cfg.thinking_effort = json_string(&val, "effortLevel");
        let skip = &["model", "effortLevel"];
        cfg.extra = super::super::schema_walker::extract_schema_fields(CLAUDE_SCHEMA, &val, skip);
        // provider_locked is always added by read_config_file; add here for parity.
        cfg.extra
            .insert("provider_locked".to_string(), "true".to_string());
        cfg
    }

    #[test]
    fn parse_model_from_settings() {
        let cfg = parse_settings(r#"{"model": "claude-sonnet-4-20250514"}"#);
        assert_eq!(cfg.model.as_deref(), Some("claude-sonnet-4-20250514"));
    }

    #[test]
    fn effort_level_maps_to_thinking_effort() {
        let cfg = parse_settings(r#"{"effortLevel": "high"}"#);
        assert_eq!(cfg.thinking_effort.as_deref(), Some("high"));
        // effortLevel must NOT appear in extra (it's in the skip list)
        assert!(!cfg.extra.contains_key("effortLevel"));
    }

    #[test]
    fn always_thinking_enabled_appears_in_extra() {
        let cfg = parse_settings(r#"{"alwaysThinkingEnabled": true}"#);
        assert_eq!(
            cfg.extra.get("alwaysThinkingEnabled").map(|s| s.as_str()),
            Some("true"),
            "alwaysThinkingEnabled should appear in extra"
        );
    }

    #[test]
    fn env_vars_flattened_in_extra() {
        let cfg = parse_settings(
            r#"{"env": {"CLAUDE_CODE_EFFORT_LEVEL": "high", "ANTHROPIC_MODEL": "claude-opus-4"}}"#,
        );
        assert_eq!(
            cfg.extra
                .get("env.CLAUDE_CODE_EFFORT_LEVEL")
                .map(|s| s.as_str()),
            Some("high"),
            "env.CLAUDE_CODE_EFFORT_LEVEL should appear in extra"
        );
        assert_eq!(
            cfg.extra.get("env.ANTHROPIC_MODEL").map(|s| s.as_str()),
            Some("claude-opus-4"),
            "env.ANTHROPIC_MODEL should appear in extra"
        );
    }

    #[test]
    fn enabled_plugins_formats_as_item_count() {
        // enabledPlugins is an object in the schema — walker flattens one level.
        // Each plugin entry is a value (bool or array), so they appear as subkeys.
        let cfg = parse_settings(
            r#"{"enabledPlugins": {"plugin-a": true, "plugin-b": true}}"#,
        );
        // The walker flattens one level: enabledPlugins.plugin-a = "true"
        assert!(
            cfg.extra.contains_key("enabledPlugins.plugin-a")
                || cfg.extra.contains_key("enabledPlugins.plugin-b"),
            "enabledPlugins entries should appear as enabledPlugins.<name> in extra"
        );
    }

    #[test]
    fn parse_permissions_and_hooks() {
        let cfg = parse_settings(
            r#"{"permissions": {"default": "bypassPermissions"}, "hooks": {"pre-commit": {}}}"#,
        );
        // permissions is an object — flattened as permissions.default
        assert_eq!(
            cfg.extra.get("permissions.default").map(|s| s.as_str()),
            Some("bypassPermissions")
        );
        // hooks is an object — flattened one level
        assert!(
            cfg.extra.contains_key("hooks.pre-commit"),
            "hooks.pre-commit should appear in extra"
        );
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
        assert!(cfg.thinking_effort.is_none());
        assert!(cfg.system_prompt.is_none());
    }

    #[test]
    fn model_not_duplicated_in_extra() {
        let cfg = parse_settings(r#"{"model": "claude-opus-4", "effortLevel": "high"}"#);
        assert!(!cfg.extra.contains_key("model"));
        assert!(!cfg.extra.contains_key("effortLevel"));
    }
}
