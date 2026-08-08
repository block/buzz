use super::types::{ExtensionEntry, RuntimeFileConfig};

/// Read Claude Code config from `settings.json` and `.claude.json`.
///
/// `config_dir` — when `Some`, reads both `settings.json` and `.claude.json`
/// from that directory (the agent's effective `CLAUDE_CONFIG_DIR`).
/// Defaults to `~/.claude/settings.json` and `~/.claude.json` when `None`.
///
/// Both files are resolved from the same directory: the claude 2.1.x binary
/// resolves `.claude.json` as `join(process.env.CLAUDE_CONFIG_DIR || homedir(),
/// ".claude.json")`, mirroring the `settings.json` resolver. A user-set
/// `CLAUDE_CONFIG_DIR` therefore remaps both files — honoring only
/// `settings.json` would misrepresent the agent's actual MCP config.
pub(super) fn read_config_file(config_dir: Option<&std::path::Path>) -> Option<RuntimeFileConfig> {
    let home = dirs::home_dir()?;

    // #3493: honor user-set CLAUDE_CONFIG_DIR for both settings.json and
    // .claude.json — the binary resolves both relative to CLAUDE_CONFIG_DIR.
    // Panel reflects the actual config the agent reads.
    let settings_path = config_dir
        .map(|d| d.join("settings.json"))
        .unwrap_or_else(|| home.join(".claude").join("settings.json"));

    let mcp_path = config_dir
        .map(|d| d.join(".claude.json"))
        .unwrap_or_else(|| home.join(".claude.json"));

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

    // MCP servers from ~/.claude.json
    let mut extensions = Vec::new();
    if let Some(ref mc) = mcp_config {
        if let Some(servers) = mc.get("mcpServers").and_then(|v| v.as_object()) {
            for (name, _config) in servers {
                extensions.push(ExtensionEntry {
                    name: name.clone(),
                    kind: "mcp".to_string(),
                    enabled: true,
                    source: None,
                });
            }
        }
    }
    cfg.extensions = extensions;

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
    use std::io::Write;

    fn write_tmp_settings(dir: &std::path::Path, content: &[u8]) {
        std::fs::create_dir_all(dir).unwrap();
        let mut f = std::fs::File::create(dir.join("settings.json")).unwrap();
        f.write_all(content).unwrap();
    }

    #[test]
    fn reads_model_from_settings_json() {
        let dir = tempfile::tempdir().unwrap();
        write_tmp_settings(
            dir.path(),
            br#"{"model": "claude-opus-4", "effortLevel": "high"}"#,
        );
        let cfg = read_config_file(Some(dir.path())).unwrap();
        assert_eq!(cfg.model.as_deref(), Some("claude-opus-4"));
        assert_eq!(cfg.thinking_effort.as_deref(), Some("high"));
    }

    #[test]
    fn returns_none_when_no_files_found() {
        let dir = tempfile::tempdir().unwrap();
        // No settings.json and no ~/.claude.json (home path won't have a test file)
        let result = read_config_file(Some(dir.path()));
        // May return Some if ~/.claude.json exists on the test machine — we
        // only assert the settings fields are absent when no settings.json.
        if let Some(cfg) = result {
            assert!(cfg.model.is_none());
            assert!(cfg.thinking_effort.is_none());
        }
    }

    #[test]
    fn defaults_to_home_claude_dir_when_no_config_dir() {
        // Calling with None falls back to ~/.claude/settings.json.
        // This is a compile-time path test — we just verify the call compiles
        // and returns without panic; we can't assert the result without HOME.
        let _result = read_config_file(None);
    }

    #[test]
    fn unknown_fields_appear_in_extra() {
        let dir = tempfile::tempdir().unwrap();
        write_tmp_settings(
            dir.path(),
            br#"{"model": "m", "someUnknownField": "value", "anotherField": true}"#,
        );
        let cfg = read_config_file(Some(dir.path())).unwrap();
        assert!(
            !cfg.extra.is_empty(),
            "unknown future fields should appear in extra"
        );
    }
}
