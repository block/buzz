use std::path::{Path, PathBuf};

use super::types::{ExtensionEntry, RuntimeFileConfig};

/// Read Claude Code config from `settings.json` and `.claude.json`, resolved
/// against `config_dir` (the agent's effective `CLAUDE_CONFIG_DIR`, if set) or
/// `~/.claude`/`~` otherwise. Passing the correct `config_dir` for an isolated
/// agent is what keeps this from reading the operator's personal config.
pub(super) fn read_config_file(config_dir: Option<&Path>) -> Option<RuntimeFileConfig> {
    let (settings_path, mcp_path) = claude_config_paths(config_dir)?;

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

        // Config-driven extra fields — skip normalized keys to avoid double-counting.
        let skip = &["model", "effortLevel"];
        cfg.extra = super::schema_walker::extract_config_fields(s, skip);
    }

    // MCP servers from mcp_path (~/.claude.json, or <CLAUDE_CONFIG_DIR>/.claude.json)
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

    Some(cfg)
}

/// Resolve `(settings.json, .claude.json)` for `config_dir`. When
/// `config_dir` is `Some` (an explicit `CLAUDE_CONFIG_DIR`), both files live
/// directly under it — matching Claude Code's own resolution, which replaces
/// `~/.claude` *and* moves the top-level `.claude.json` inside it. Otherwise
/// falls back to the default `~/.claude/settings.json` + `~/.claude.json`.
pub(super) fn claude_config_paths(config_dir: Option<&Path>) -> Option<(PathBuf, PathBuf)> {
    match config_dir {
        Some(dir) => Some((dir.join("settings.json"), dir.join(".claude.json"))),
        None => {
            let home = dirs::home_dir()?;
            Some((
                home.join(".claude").join("settings.json"),
                home.join(".claude.json"),
            ))
        }
    }
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

    #[test]
    fn config_dir_override_reads_settings_and_mcp_from_isolated_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("settings.json"),
            r#"{"model": "isolated-model"}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join(".claude.json"),
            r#"{"mcpServers": {"isolated-server": {"command": "foo"}}}"#,
        )
        .unwrap();

        let cfg = read_config_file(Some(dir.path())).expect("config read from isolated dir");
        assert_eq!(cfg.model.as_deref(), Some("isolated-model"));
        assert_eq!(cfg.extensions.len(), 1);
        assert_eq!(cfg.extensions[0].name, "isolated-server");
    }

    #[test]
    fn config_dir_override_does_not_leak_operator_home_config() {
        // An isolated agent's CLAUDE_CONFIG_DIR pointing at an empty directory
        // must never fall back to reading the operator's ~/.claude.json, even
        // when the isolated dir has no files of its own yet.
        let dir = tempfile::tempdir().unwrap();
        let cfg = read_config_file(Some(dir.path()));
        assert!(
            cfg.is_none(),
            "empty isolated dir must not surface any config"
        );
    }

    #[test]
    fn claude_config_paths_nests_both_files_under_explicit_config_dir() {
        let dir = tempfile::tempdir().unwrap();
        let (settings_path, mcp_path) = claude_config_paths(Some(dir.path())).unwrap();
        assert_eq!(settings_path, dir.path().join("settings.json"));
        assert_eq!(mcp_path, dir.path().join(".claude.json"));
    }

    /// Parse a settings JSON string into a RuntimeFileConfig using the same
    /// logic as read_config_file but without touching the filesystem.
    fn parse_settings(json: &str) -> RuntimeFileConfig {
        let val: serde_json::Value = serde_json::from_str(json).unwrap();
        let skip = &["model", "effortLevel"];
        RuntimeFileConfig {
            model: json_string(&val, "model"),
            thinking_effort: json_string(&val, "effortLevel"),
            extra: super::super::schema_walker::extract_config_fields(&val, skip),
            ..Default::default()
        }
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
    fn arbitrary_env_var_surfaced_without_schema() {
        // Config-driven: any env var the user has set appears, even if no schema
        // defines it — this is the core benefit over the schema-driven approach.
        let cfg = parse_settings(r#"{"env": {"MY_CUSTOM_VAR": "hello"}}"#);
        assert_eq!(
            cfg.extra.get("env.MY_CUSTOM_VAR").map(|s| s.as_str()),
            Some("hello"),
            "arbitrary env vars should appear in extra"
        );
    }

    #[test]
    fn enabled_plugins_flattened_in_extra() {
        let cfg = parse_settings(r#"{"enabledPlugins": {"plugin-a": true, "plugin-b": true}}"#);
        // Walker flattens one level: enabledPlugins.plugin-a = "true"
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
        // hooks.pre-commit is an empty object — emits placeholder
        assert_eq!(
            cfg.extra.get("hooks.pre-commit").map(|s| s.as_str()),
            Some("{...}")
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

    #[test]
    fn unknown_future_field_appears_in_extra() {
        // Config-driven: any field the user has set appears, even if we've never
        // heard of it. No schema gate.
        let cfg = parse_settings(r#"{"someNewClaudeField": "value"}"#);
        assert_eq!(
            cfg.extra.get("someNewClaudeField").map(|s| s.as_str()),
            Some("value"),
            "unknown future fields should appear in extra"
        );
    }
}
